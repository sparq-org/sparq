//! sparq-gpu: opt-in wgpu compute kernels for the sparq RDF engine (roadmap T24d).
//!
//! This crate exists to answer ONE question with measurements, not aspiration:
//! **where (if anywhere) does a GPU beat the CPU for sparq's hot paths once the
//! host→device transfer tax is charged honestly?** See `README.md` for the
//! measured verdict and `examples/gpu_bench.rs` for the experiment.
//!
//! Three kernels, chosen to span the compute-intensity axis:
//!
//! 1. [`Gpu::filter_count_u32`] / [`Gpu::filter_count_f64_gt`] — FILTER
//!    evaluation over a resident column (sparq's permutation-index object column
//!    / dense numeric cache). **Compute-light**: ~1 compare per 4–8 bytes read —
//!    the PCIe-dominated case.
//! 2. [`Gpu::hash_probe`] — hash-join probe of a resident probe column against a
//!    resident open-addressing build table. **Compute-medium**: hashing + a
//!    data-dependent probe walk per element.
//! 3. [`Gpu::group_aggregate`] — COUNT + SUM GROUP BY over resident key/value
//!    columns via two-level (workgroup-shared, then global) atomics with u64
//!    emulation. **Compute-dense** for a scan shape: every element does atomic
//!    read-modify-writes, and only `3·G` words ever come back to the host.
//!
//! Design constraints baked in:
//! - Columns are plain `&[u32]` / `&[f64]` — the exact shape of sparq-core's
//!   id columns and numeric cache; no engine dependency needed to measure.
//! - All kernels reduce on-device and read back O(1)/O(G) bytes, the best case
//!   for the GPU (materialising selected rows would add a device→host stream
//!   and can only make the GPU look worse — see README).
//! - [`Gpu::new`] returns `None` when no adapter exists, so CI (no GPU) skips
//!   gracefully.
//! - WGSL has **no f64**: the f64 filter compares IEEE-754 *bit patterns* mapped
//!   to a monotonic u64 key (sign-flip trick) — exact, no precision loss, but a
//!   real portability cost worth knowing about (see README §limits).
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

use bytemuck::{Pod, Zeroable};

pub mod cpu;

const WG: u32 = 256;
/// Sentinel key marking an empty hash-table slot (build keys must avoid it).
pub const EMPTY_KEY: u32 = u32::MAX;
/// The group-aggregate kernel tiles per-group accumulators in workgroup shared
/// memory; this caps the supported group count (512 × 3 × 4 B = 6 KiB shared,
/// under WebGPU's 16 KiB minimum guarantee).
pub const MAX_GROUPS: u32 = 512;

/// Upper bound on how long [`Gpu`] waits for one kernel submission before
/// giving up with [`GpuError::Stalled`] instead of blocking the host thread
/// forever. Every kernel is O(n) with bounded per-element work, so a healthy
/// device finishes far inside this; hitting it means a wedged device.
/// GitHub #4603.
pub const POLL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// A GROUP BY key was `>= groups` (the documented `keys[i] < groups`
/// precondition was violated). Returned by both [`Gpu::group_aggregate`] and
/// the CPU oracle [`cpu::group_aggregate`], so the two sides agree on malformed
/// input instead of the GPU silently dropping the row while the CPU panics.
/// GitHub #4603.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupKeyOutOfRange;

impl std::fmt::Display for GroupKeyOutOfRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GROUP BY key out of range: every key must be < groups")
    }
}

impl std::error::Error for GroupKeyOutOfRange {}

/// Why a [`Gpu`] kernel call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuError {
    /// The device did not finish a submission within [`POLL_TIMEOUT`] (or the
    /// poll itself failed). The [`Gpu`] is now permanently stalled: every later
    /// kernel call returns this error without touching the device, and dropping
    /// the [`Gpu`] leaks its device and queue instead of waiting on them (see
    /// the `Drop` impl). Recreate a [`Gpu`] to retry.
    Stalled,
    /// See [`GroupKeyOutOfRange`] (only [`Gpu::group_aggregate`]).
    GroupKeyOutOfRange,
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::Stalled => write!(
                f,
                "GPU device stalled: a submission did not complete within {POLL_TIMEOUT:?}"
            ),
            GpuError::GroupKeyOutOfRange => GroupKeyOutOfRange.fmt(f),
        }
    }
}

impl std::error::Error for GpuError {}

impl From<GroupKeyOutOfRange> for GpuError {
    fn from(_: GroupKeyOutOfRange) -> Self {
        GpuError::GroupKeyOutOfRange
    }
}

/// Keeps `handle`'s shared resource alive for the rest of the process by
/// forgetting one extra strong reference to it, so its destructor never runs.
/// Used on wgpu's `Device`/`Queue` (both `Arc`-backed `Clone` handles) once a
/// submission has timed out — see `impl Drop for Gpu`.
fn leak_handle<T: Clone>(handle: &T) {
    std::mem::forget(handle.clone());
}

/// Validates a host-built open-addressing table before it reaches the device:
/// the slot count must be a non-zero power of two (the kernel masks with
/// `len - 1`) and at least one slot must be `EMPTY_KEY`, so every probe walk has
/// a terminating sentinel. Panics otherwise. GitHub #4603.
fn check_table(slots: &[[u32; 2]]) {
    assert!(
        slots.len().is_power_of_two(),
        "hash table slot count must be a non-zero power of two, got {}",
        slots.len()
    );
    assert!(
        u32::try_from(slots.len()).is_ok(),
        "hash table slot count must fit in u32"
    );
    assert!(
        slots.iter().any(|s| s[0] == EMPTY_KEY),
        "hash table must contain at least one EMPTY_KEY slot (load factor < 1)"
    );
}

/// The 32-bit mixer used by both the CPU reference and the WGSL kernel (they
/// must agree bit-for-bit so both sides walk identical probe sequences).
#[inline]
pub fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

#[cfg(test)]
mod tests {
    use super::{check_table, hash32, leak_handle, GpuError, GroupKeyOutOfRange, EMPTY_KEY};

    #[test]
    fn leak_handle_keeps_the_shared_resource_alive() {
        let handle = std::rc::Rc::new(());
        let weak = std::rc::Rc::downgrade(&handle);
        leak_handle(&handle);
        drop(handle);
        // The destructor of the shared value never runs: a stalled wgpu queue is
        // never torn down (and so never waited on).
        assert!(weak.upgrade().is_some());
    }

    #[test]
    fn gpu_error_display_and_from() {
        assert_eq!(
            GpuError::from(GroupKeyOutOfRange),
            GpuError::GroupKeyOutOfRange
        );
        assert_eq!(
            GpuError::GroupKeyOutOfRange.to_string(),
            GroupKeyOutOfRange.to_string()
        );
        assert!(GpuError::Stalled.to_string().contains("stalled"));
    }

    #[test]
    fn check_table_accepts_a_built_table() {
        check_table(&crate::cpu::build_hash_table(&[1, 2, 3], &[4, 5, 6]));
    }

    #[test]
    #[should_panic(expected = "EMPTY_KEY slot")]
    fn check_table_rejects_a_full_table() {
        check_table(&[[1, 0], [2, 0], [3, 0], [4, 0]]);
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn check_table_rejects_a_non_power_of_two_table() {
        check_table(&[[EMPTY_KEY, 0], [EMPTY_KEY, 0], [EMPTY_KEY, 0]]);
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn check_table_rejects_an_empty_table() {
        check_table(&[]);
    }

    // [GPT-5.6] sq-pz5rf: Exercise the mixer directly, including values around
    // the hash-table sentinel and a bounded sample of the full u32 domain.
    #[test]
    fn hash32_is_deterministic_and_total_for_sampled_inputs() {
        let edge_inputs = [
            0,
            1,
            u32::MAX,
            u32::MAX - 1,
            u32::MAX / 2,
            EMPTY_KEY - 2,
            EMPTY_KEY - 1,
        ];

        for input in edge_inputs.into_iter().chain(0..10_000) {
            assert_eq!(hash32(input), hash32(input), "input {input}");
        }
    }
}

// ---------------------------------------------------------------------------
// WGSL kernels
// ---------------------------------------------------------------------------

/// FILTER `lo <= v < hi` + count over a u32 column. Per-workgroup shared atomic,
/// flushed once per workgroup to a single global counter. 2D-tiled dispatch
/// (WebGPU caps each dispatch dimension at 65535 workgroups).
const WGSL_FILTER_U32: &str = r#"
struct Params { n: u32, lo: u32, hi: u32, _pad: u32 };
@group(0) @binding(0) var<storage, read>       col:    array<u32>;
@group(0) @binding(1) var<storage, read_write> result: array<atomic<u32>>;
@group(0) @binding(2) var<uniform>             params: Params;

var<workgroup> wg_count: atomic<u32>;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_id) lid: vec3<u32>,
        @builtin(workgroup_id)        wid: vec3<u32>,
        @builtin(num_workgroups)      ngw: vec3<u32>) {
    if (lid.x == 0u) { atomicStore(&wg_count, 0u); }
    workgroupBarrier();
    let i = (wid.y * ngw.x + wid.x) * 256u + lid.x;
    if (i < params.n) {
        let v = col[i];
        if (v >= params.lo && v < params.hi) { atomicAdd(&wg_count, 1u); }
    }
    workgroupBarrier();
    if (lid.x == 0u) {
        let c = atomicLoad(&wg_count);
        if (c != 0u) { atomicAdd(&result[0], c); }
    }
}
"#;

/// FILTER `v > t` + count over an f64 column — WGSL has no f64, so each element
/// is its raw IEEE-754 bits as vec2<u32> (x = low word, y = high word) and the
/// comparison maps bits to a monotonic key: negative => ~bits, else bits ^ sign
/// bit. Exact for all non-NaN values; NaNs are detected and excluded (matching
/// `v > t` semantics). The threshold arrives pre-mapped from the host.
const WGSL_FILTER_F64: &str = r#"
struct Params { n: u32, key_lo: u32, key_hi: u32, _pad: u32 };
@group(0) @binding(0) var<storage, read>       col:    array<vec2<u32>>;
@group(0) @binding(1) var<storage, read_write> result: array<atomic<u32>>;
@group(0) @binding(2) var<uniform>             params: Params;

var<workgroup> wg_count: atomic<u32>;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_id) lid: vec3<u32>,
        @builtin(workgroup_id)        wid: vec3<u32>,
        @builtin(num_workgroups)      ngw: vec3<u32>) {
    if (lid.x == 0u) { atomicStore(&wg_count, 0u); }
    workgroupBarrier();
    let i = (wid.y * ngw.x + wid.x) * 256u + lid.x;
    if (i < params.n) {
        let v = col[i];
        let lo = v.x;
        let hi = v.y;
        let exp = (hi >> 20u) & 0x7FFu;
        let is_nan = exp == 0x7FFu && (((hi & 0xFFFFFu) | lo) != 0u);
        var khi: u32;
        var klo: u32;
        if ((hi & 0x80000000u) != 0u) { khi = ~hi; klo = ~lo; }
        else                          { khi = hi ^ 0x80000000u; klo = lo; }
        let gt = khi > params.key_hi || (khi == params.key_hi && klo > params.key_lo);
        if (!is_nan && gt) { atomicAdd(&wg_count, 1u); }
    }
    workgroupBarrier();
    if (lid.x == 0u) {
        let c = atomicLoad(&wg_count);
        if (c != 0u) { atomicAdd(&result[0], c); }
    }
}
"#;

/// Hash-join probe: for each probe id, walk the resident open-addressing table
/// (linear probing, load factor <= 0.5, EMPTY_KEY sentinel) counting matches and
/// summing matched payloads. Both u64 results (match count and payload sum) are
/// emulated with a lo-word atomic + wrap-detected carry into a hi-word atomic
/// (WGSL atomics are 32-bit): duplicate build keys make >u32::MAX matches
/// possible on a high-fanout join, so the count gets the same treatment as the
/// sum.
const WGSL_HASH_PROBE: &str = r#"
struct Params { n: u32, mask: u32, _p0: u32, _p1: u32 };
@group(0) @binding(0) var<storage, read>       table:  array<vec2<u32>>; // x=key, y=payload
@group(0) @binding(1) var<storage, read>       probe:  array<u32>;
@group(0) @binding(2) var<storage, read_write> result: array<atomic<u32>>; // [m_lo, m_hi, sum_lo, sum_hi]
@group(0) @binding(3) var<uniform>             params: Params;

var<workgroup> wg_m:   atomic<u32>;
var<workgroup> wg_mhi: atomic<u32>;
var<workgroup> wg_lo:  atomic<u32>;
var<workgroup> wg_hi:  atomic<u32>;

fn hash32(x0: u32) -> u32 {
    var x = x0;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_id) lid: vec3<u32>,
        @builtin(workgroup_id)        wid: vec3<u32>,
        @builtin(num_workgroups)      ngw: vec3<u32>) {
    if (lid.x == 0u) {
        atomicStore(&wg_m, 0u);
        atomicStore(&wg_mhi, 0u);
        atomicStore(&wg_lo, 0u);
        atomicStore(&wg_hi, 0u);
    }
    workgroupBarrier();
    let i = (wid.y * ngw.x + wid.x) * 256u + lid.x;
    if (i < params.n) {
        let k = probe[i];
        var slot = hash32(k) & params.mask;
        // Bounded walk: at most mask + 1 slots (every slot once), so a table
        // with no EMPTY_KEY slot cannot spin forever (GitHub #4603).
        var steps = 0u;
        loop {
            if (steps > params.mask) { break; }
            steps += 1u;
            let e = table[slot];
            if (e.x == 0xFFFFFFFFu) { break; }
            if (e.x == k) {
                let oldm = atomicAdd(&wg_m, 1u);
                if (oldm + 1u < oldm) { atomicAdd(&wg_mhi, 1u); }
                let old = atomicAdd(&wg_lo, e.y);
                if (old + e.y < old) { atomicAdd(&wg_hi, 1u); }
            }
            slot = (slot + 1u) & params.mask;
        }
    }
    workgroupBarrier();
    if (lid.x == 0u) {
        let m = atomicLoad(&wg_m);
        if (m != 0u) {
            let oldm = atomicAdd(&result[0], m);
            if (oldm + m < oldm) { atomicAdd(&result[1], 1u); }
        }
        let mhi = atomicLoad(&wg_mhi);
        if (mhi != 0u) { atomicAdd(&result[1], mhi); }
        let lo = atomicLoad(&wg_lo);
        if (lo != 0u) {
            let old = atomicAdd(&result[2], lo);
            if (old + lo < old) { atomicAdd(&result[3], 1u); }
        }
        let hi = atomicLoad(&wg_hi);
        if (hi != 0u) { atomicAdd(&result[3], hi); }
    }
}
"#;

/// COUNT + SUM GROUP BY: keys must already be in [0, g), g <= MAX_GROUPS. A key
/// `>= g` is not accumulated; it sets the out-of-range flag word `out[3g]`, which
/// the host turns into [`GroupKeyOutOfRange`] (GitHub #4603).
/// Two-level atomics: every thread accumulates into workgroup-shared per-group
/// counters (the compute-dense part), then each workgroup flushes its g partial
/// rows to global atomics once. u64 sums via the same lo/carry/hi emulation.
const WGSL_GROUP_AGG: &str = r#"
const MAX_G: u32 = 512u;
struct Params { n: u32, g: u32, _p0: u32, _p1: u32 };
@group(0) @binding(0) var<storage, read>       keys: array<u32>;
@group(0) @binding(1) var<storage, read>       vals: array<u32>;
@group(0) @binding(2) var<storage, read_write> out:  array<atomic<u32>>; // [count;g][sum_lo;g][sum_hi;g][oob_flag]
@group(0) @binding(3) var<uniform>             params: Params;

var<workgroup> s_cnt: array<atomic<u32>, MAX_G>;
var<workgroup> s_lo:  array<atomic<u32>, MAX_G>;
var<workgroup> s_hi:  array<atomic<u32>, MAX_G>;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_id) lid: vec3<u32>,
        @builtin(workgroup_id)        wid: vec3<u32>,
        @builtin(num_workgroups)      ngw: vec3<u32>) {
    var g = lid.x;
    while (g < params.g) {
        atomicStore(&s_cnt[g], 0u);
        atomicStore(&s_lo[g], 0u);
        atomicStore(&s_hi[g], 0u);
        g += 256u;
    }
    workgroupBarrier();
    let i = (wid.y * ngw.x + wid.x) * 256u + lid.x;
    if (i < params.n) {
        let k = keys[i];
        let v = vals[i];
        if (k < params.g) {
            atomicAdd(&s_cnt[k], 1u);
            let old = atomicAdd(&s_lo[k], v);
            if (old + v < old) { atomicAdd(&s_hi[k], 1u); }
        } else {
            atomicOr(&out[3u * params.g], 1u);
        }
    }
    workgroupBarrier();
    g = lid.x;
    while (g < params.g) {
        let c = atomicLoad(&s_cnt[g]);
        if (c != 0u) { atomicAdd(&out[g], c); }
        let lo = atomicLoad(&s_lo[g]);
        if (lo != 0u) {
            let old = atomicAdd(&out[params.g + g], lo);
            if (old + lo < old) { atomicAdd(&out[2u * params.g + g], 1u); }
        }
        let hi = atomicLoad(&s_hi[g]);
        if (hi != 0u) { atomicAdd(&out[2u * params.g + g], hi); }
        g += 256u;
    }
}
"#;

// ---------------------------------------------------------------------------
// Host side
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

/// A device-resident u32 column (e.g. a permutation-index object column, a
/// probe-side id column, or GROUP BY keys/values).
pub struct ColU32 {
    buf: wgpu::Buffer,
    pub len: u32,
}

/// A device-resident f64 column (the dense numeric cache), stored as raw
/// IEEE-754 bits (the kernel does bit-pattern comparison; upload is a memcpy).
pub struct ColF64 {
    buf: wgpu::Buffer,
    pub len: u32,
}

/// A device-resident open-addressing hash table of (key, payload) pairs, built
/// on the host by [`cpu::build_hash_table`].
pub struct HashTable {
    buf: wgpu::Buffer,
    /// Slot count minus one (capacity is a power of two).
    pub mask: u32,
}

/// Maps an f64's bits to a u64 whose unsigned order matches IEEE-754 `<` on
/// non-NaN values (the classic sign-flip total-order trick).
#[inline]
fn f64_order_key(v: f64) -> u64 {
    let bits = v.to_bits();
    if bits & 0x8000_0000_0000_0000 != 0 {
        !bits
    } else {
        bits ^ 0x8000_0000_0000_0000
    }
}

/// A wgpu device + the four compiled kernel pipelines.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    /// The adapter's max storage-buffer binding — resident columns must fit.
    pub max_storage_bytes: u64,
    filter_u32: wgpu::ComputePipeline,
    filter_f64: wgpu::ComputePipeline,
    hash_probe: wgpu::ComputePipeline,
    group_agg: wgpu::ComputePipeline,
    /// Set once a submission fails to complete within [`POLL_TIMEOUT`].
    stalled: std::sync::atomic::AtomicBool,
}

/// GitHub #4603: a timed-out submission may still be pending on the device,
/// and wgpu-core 30's `Drop for Queue` calls the HAL's `wait_for_idle()` with
/// no timeout — so a plain drop of a stalled [`Gpu`] (including during panic
/// unwinding) would block forever on the very submission we gave up on. Once
/// stalled, we deliberately leak the device and queue instead: their
/// destructors never run, and the driver reclaims them at process exit. A
/// healthy [`Gpu`] drops normally.
impl Drop for Gpu {
    fn drop(&mut self) {
        if self.stalled.load(std::sync::atomic::Ordering::Acquire) {
            leak_handle(&self.queue);
            leak_handle(&self.device);
        }
    }
}

impl Gpu {
    /// Initialises the first available high-performance adapter. Returns `None`
    /// when no usable GPU/driver exists (e.g. CI) so callers can skip cleanly.
    pub fn new() -> Option<Gpu> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Option<Gpu> {
        let instance = wgpu::Instance::default();
        // [OPUS-4.8] wgpu 22 -> 29 migration (sq-5z08): `request_adapter` now
        // returns `Result` (was `Option` in 22); `.ok()?` keeps the
        // "no adapter ⇒ None" contract callers rely on.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .ok()?;
        let info = adapter.get_info();
        let limits = adapter.limits();
        // wgpu 29: `max_storage_buffer_binding_size` is already `u64` (was `u32`
        // in 22, where this needed a `u64::from`).
        let max_storage_bytes = limits.max_storage_buffer_binding_size;
        // wgpu 29: `request_device` takes only the descriptor (the trailing trace
        // `Option` arg was folded into `DeviceDescriptor::trace`), and the
        // descriptor gained `experimental_features` + `trace` (both `Default`).
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("sparq-gpu"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .ok()?;

        let pipeline = |label: &str, src: &str| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &module,
                // wgpu 29: `entry_point` is now `Option<&str>` (was `&str`).
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        };

        Some(Gpu {
            filter_u32: pipeline("filter_u32", WGSL_FILTER_U32),
            filter_f64: pipeline("filter_f64", WGSL_FILTER_F64),
            hash_probe: pipeline("hash_probe", WGSL_HASH_PROBE),
            group_agg: pipeline("group_agg", WGSL_GROUP_AGG),
            device,
            queue,
            info,
            max_storage_bytes,
            stalled: std::sync::atomic::AtomicBool::new(false),
        })
    }

    // -- column upload / overwrite (overwrite = the measured "transfer tax") --

    pub fn upload_u32(&self, data: &[u32]) -> ColU32 {
        ColU32 {
            buf: self.storage_buffer(bytemuck::cast_slice(data)),
            len: data.len() as u32,
        }
    }

    pub fn upload_f64(&self, data: &[f64]) -> ColF64 {
        ColF64 {
            buf: self.storage_buffer(bytemuck::cast_slice(data)),
            len: data.len() as u32,
        }
    }

    /// Uploads a host-built open-addressing table (see [`cpu::build_hash_table`]).
    ///
    /// # Panics
    /// If the slot count is not a non-zero power of two, or no slot is
    /// `EMPTY_KEY` (a full table would give the probe walk no sentinel).
    pub fn upload_table(&self, slots: &[[u32; 2]]) -> HashTable {
        check_table(slots);
        HashTable {
            buf: self.storage_buffer(bytemuck::cast_slice(slots)),
            mask: (slots.len() - 1) as u32,
        }
    }

    /// Overwrites a resident u32 column — the host→device transfer the e2e
    /// benchmark legs charge. (The copy is enqueued and lands at next submit.)
    pub fn write_u32(&self, col: &ColU32, data: &[u32]) {
        assert_eq!(col.len as usize, data.len());
        self.queue
            .write_buffer(&col.buf, 0, bytemuck::cast_slice(data));
    }

    pub fn write_f64(&self, col: &ColF64, data: &[f64]) {
        assert_eq!(col.len as usize, data.len());
        self.queue
            .write_buffer(&col.buf, 0, bytemuck::cast_slice(data));
    }

    pub fn write_table(&self, table: &HashTable, slots: &[[u32; 2]]) {
        assert_eq!(table.mask as usize + 1, slots.len());
        // No `check_table` scan here: this is the timed host->device refill in the
        // e2e benchmark legs, and an extra O(n) host pass would skew them. A full
        // table written here still cannot hang — the probe walk is bounded.
        self.queue
            .write_buffer(&table.buf, 0, bytemuck::cast_slice(slots));
    }

    fn storage_buffer(&self, contents: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sparq-gpu column"),
                contents,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
    }

    // -- kernels --

    /// Counts elements with `lo <= v < hi` in a resident u32 column.
    pub fn filter_count_u32(&self, col: &ColU32, lo: u32, hi: u32) -> Result<u64, GpuError> {
        let r = self.run(
            &self.filter_u32,
            &[col.buf.as_entire_binding()],
            Params {
                a: col.len,
                b: lo,
                c: hi,
                d: 0,
            },
            col.len,
            1,
        )?;
        Ok(u64::from(r[0]))
    }

    /// Counts elements with `v > t` (NaN-excluding, IEEE semantics, exact) in a
    /// resident f64 column.
    pub fn filter_count_f64_gt(&self, col: &ColF64, t: f64) -> Result<u64, GpuError> {
        if t.is_nan() {
            return Ok(0); // v > NaN is false for every v; no dispatch needed
        }
        let key = f64_order_key(if t == 0.0 { 0.0 } else { t }); // canonicalise -0.0
        let r = self.run(
            &self.filter_f64,
            &[col.buf.as_entire_binding()],
            Params {
                a: col.len,
                b: key as u32,
                c: (key >> 32) as u32,
                d: 0,
            },
            col.len,
            1,
        )?;
        Ok(u64::from(r[0]))
    }

    /// Probes every id in `probe` against the resident `table`; returns
    /// (number of matches, exact u64 sum of matched payloads). Both are exact
    /// u64s on the device (lo/carry/hi atomic emulation) — duplicate build keys
    /// can push a high-fanout join past u32::MAX matches.
    pub fn hash_probe(&self, table: &HashTable, probe: &ColU32) -> Result<(u64, u64), GpuError> {
        let r = self.run(
            &self.hash_probe,
            &[table.buf.as_entire_binding(), probe.buf.as_entire_binding()],
            Params {
                a: probe.len,
                b: table.mask,
                c: 0,
                d: 0,
            },
            probe.len,
            4,
        )?;
        Ok((
            (u64::from(r[1]) << 32) | u64::from(r[0]),
            (u64::from(r[3]) << 32) | u64::from(r[2]),
        ))
    }

    /// COUNT + SUM GROUP BY over resident columns. `groups <= MAX_GROUPS`.
    /// Returns `(count, sum)` per group, or [`GpuError::GroupKeyOutOfRange`] if
    /// any `keys[i] >= groups` (detected on the device, no host scan).
    pub fn group_aggregate(
        &self,
        keys: &ColU32,
        vals: &ColU32,
        groups: u32,
    ) -> Result<Vec<(u64, u64)>, GpuError> {
        assert!(
            (1..=MAX_GROUPS).contains(&groups),
            "groups must be in 1..={MAX_GROUPS}"
        );
        assert_eq!(keys.len, vals.len);
        let r = self.run(
            &self.group_agg,
            &[keys.buf.as_entire_binding(), vals.buf.as_entire_binding()],
            Params {
                a: keys.len,
                b: groups,
                c: 0,
                d: 0,
            },
            keys.len,
            3 * groups as usize + 1,
        )?;
        let g = groups as usize;
        if r[3 * g] != 0 {
            return Err(GpuError::GroupKeyOutOfRange);
        }
        Ok((0..g)
            .map(|i| {
                let count = u64::from(r[i]);
                let sum = (u64::from(r[2 * g + i]) << 32) | u64::from(r[g + i]);
                (count, sum)
            })
            .collect())
    }

    /// Shared dispatch path: binds `inputs` at 0.., a zero-initialised result
    /// buffer of `result_words` u32s next, the uniform params last; dispatches a
    /// 2D-tiled grid covering `n` elements; reads the result back synchronously.
    /// Fails with [`GpuError::Stalled`] (and marks `self` stalled) if the
    /// submission does not complete within [`POLL_TIMEOUT`].
    fn run(
        &self,
        pipeline: &wgpu::ComputePipeline,
        inputs: &[wgpu::BindingResource],
        params: Params,
        n: u32,
        result_words: usize,
    ) -> Result<Vec<u32>, GpuError> {
        use std::sync::atomic::Ordering;
        use wgpu::util::DeviceExt;
        if self.stalled.load(Ordering::Acquire) {
            return Err(GpuError::Stalled);
        }
        let result_bytes = (result_words * 4) as u64;
        // Freshly created buffers are zero-initialised per the WebGPU spec.
        let result = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("result"),
            size: result_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: result_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let mut entries: Vec<wgpu::BindGroupEntry> = inputs
            .iter()
            .enumerate()
            .map(|(i, r)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: r.clone(),
            })
            .collect();
        entries.push(wgpu::BindGroupEntry {
            binding: inputs.len() as u32,
            resource: result.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: inputs.len() as u32 + 1,
            resource: params_buf.as_entire_binding(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });

        // 2D-tile the workgroup grid: WebGPU caps each dimension at 65535.
        let n_groups = n.div_ceil(WG).max(1);
        let gx = n_groups.min(65535);
        let gy = n_groups.div_ceil(gx);

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            cp.set_pipeline(pipeline);
            cp.set_bind_group(0, &bind, &[]);
            cp.dispatch_workgroups(gx, gy, 1);
        }
        enc.copy_buffer_to_buffer(&result, 0, &readback, 0, result_bytes);
        let submission = self.queue.submit(Some(enc.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        // wgpu 29: `Maintain::Wait` became `PollType::wait_indefinitely()` (block
        // on the most recent submission, no timeout — same semantics as before),
        // and `poll` now returns a `Result` (Err only on device loss / timeout).
        // wgpu 30: `get_mapped_range()` now returns `Result<BufferView, MapRangeError>`
        // instead of `BufferView` directly — unwrap because a mapping error here is
        // a GPU protocol violation (we waited for the map to succeed above).
        // GitHub #4603: wait for this submission with a bound ([`POLL_TIMEOUT`])
        // rather than `wait_indefinitely()`, so a wedged device fails instead of
        // hanging the host thread. The failure is a returned error, not a panic:
        // we mark `self` stalled first so neither the caller's eventual drop nor
        // a panic unwind runs wgpu's unbounded queue-idle wait (see `Drop`).
        if self
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(POLL_TIMEOUT),
            })
            .is_err()
        {
            self.stalled.store(true, Ordering::Release);
            return Err(GpuError::Stalled);
        }
        rx.recv()
            .expect("map_async callback dropped")
            .expect("readback map failed");
        let out: Vec<u32> =
            bytemuck::cast_slice(&slice.get_mapped_range().expect("get_mapped_range failed"))
                .to_vec();
        readback.unmap();
        Ok(out)
    }
}

// The WGSL result-buffer bindings above are written through the kernels' result
// arrays; bind-group layout note: `inputs` occupy bindings 0..k, the result
// buffer binding k, the uniform params binding k+1 — matching each shader.
