//! [GPT-6 Astra] Bench-only System wrapper, following bench/alloc-track.
//! Requested live bytes exclude allocator metadata and transient realloc internals.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

struct Counting;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

fn added(bytes: usize) {
    let live = LIVE.fetch_add(bytes as u64, Relaxed) + bytes as u64;
    if ACTIVE.load(Relaxed) {
        PEAK.fetch_max(live, Relaxed);
    }
}

// SAFETY: All pointer/layout operations are forwarded unchanged to System.
// Atomics allocate nothing, never dereference pointers and never unwind.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The GlobalAlloc caller supplies a valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            added(layout.size());
            if ACTIVE.load(Relaxed) {
                ALLOCS.fetch_add(1, Relaxed);
                BYTES.fetch_add(layout.size() as u64, Relaxed);
            }
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The GlobalAlloc caller supplies a valid layout.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            added(layout.size());
            if ACTIVE.load(Relaxed) {
                ALLOCS.fetch_add(1, Relaxed);
                BYTES.fetch_add(layout.size() as u64, Relaxed);
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies this allocation's original pointer/layout.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as u64, Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a live allocation and valid nonzero new size.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                added(new_size - layout.size());
            } else {
                LIVE.fetch_sub((layout.size() - new_size) as u64, Relaxed);
            }
            if ACTIVE.load(Relaxed) {
                REALLOCS.fetch_add(1, Relaxed);
                // Full new request size, not merely the growth in live bytes.
                BYTES.fetch_add(new_size as u64, Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Begin a window while the benchmark and its initialized Rayon pool are idle.
pub fn begin() -> u64 {
    assert!(!ACTIVE.swap(false, Relaxed));
    let baseline = LIVE.load(Relaxed);
    PEAK.store(baseline, Relaxed);
    ALLOCS.store(0, Relaxed);
    REALLOCS.store(0, Relaxed);
    BYTES.store(0, Relaxed);
    ACTIVE.store(true, Relaxed);
    baseline
}

/// Stop the window before formatting output or checking returned query results.
pub fn end(baseline: u64) -> (u64, u64, u64, u64, u64) {
    ACTIVE.store(false, Relaxed);
    (
        ALLOCS.load(Relaxed),
        REALLOCS.load(Relaxed),
        BYTES.load(Relaxed),
        PEAK.load(Relaxed).saturating_sub(baseline),
        LIVE.load(Relaxed),
    )
}

pub fn calibrate() {
    let baseline = begin();
    // SAFETY: Each successful allocation is used only with its matching layout;
    // realloc transfers ownership on success. No allocated byte is dereferenced.
    unsafe {
        let old = Layout::from_size_align(128, 8).unwrap();
        let zero = Layout::from_size_align(64, 8).unwrap();
        let p = ALLOCATOR.alloc(old);
        let q = ALLOCATOR.alloc_zeroed(zero);
        assert!(!p.is_null() && !q.is_null());
        let p = ALLOCATOR.realloc(p, old, 256);
        assert!(!p.is_null());
        ALLOCATOR.dealloc(p, Layout::from_size_align(256, 8).unwrap());
        ALLOCATOR.dealloc(q, zero);
    }
    assert_eq!(end(baseline), (2, 1, 448, 320, baseline));
}
