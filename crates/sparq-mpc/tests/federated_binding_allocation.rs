//! Allocation guards for public capacity and proof-length refusals. One test in
//! its own binary keeps the process-wide counters isolated from other tests.

#![deny(clippy::undocumented_unsafe_blocks)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use sparq_mpc::federated_binding::{
    BindingError, FederatedStatement, FieldWord, HolderSegment, ValidityWindow, VerifierChallenge,
    MAX_HOLDER_ROW_CAPACITY, WORD_BYTES,
};
use sparq_mpc::partial::HolderId;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn allocated(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::SeqCst) + bytes;
    PEAK.fetch_max(live, Ordering::SeqCst);
}

// SAFETY: all allocation operations forward identical arguments to System.
// Only atomic byte counters are updated; no pointer is retained or dereferenced.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's layout contract is forwarded unchanged to System.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: layout is unchanged from the GlobalAlloc call.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }

    // SAFETY: all allocations use System, with the same pointer and layout.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr and layout satisfy the caller's deallocation contract.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }

    // SAFETY: the pointer, layout, and new size are forwarded unchanged.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: all arguments satisfy the caller's reallocation contract.
        let next = unsafe { System.realloc(ptr, layout, new_size) };
        if !next.is_null() {
            if new_size >= layout.size() {
                allocated(new_size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::SeqCst);
            }
        }
        next
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn with_peak<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let result = f();
    let peak = PEAK.load(Ordering::SeqCst).saturating_sub(base);
    (result, peak)
}

#[test]
fn capacity_and_length_refusals_do_not_allocate_public_input_buffers() {
    // Prepare owned arguments before measuring: rejecting a declared capacity
    // needs no padding allocation, even with a disclosed row present.
    let holder = HolderId::new("alice");
    let commitments = vec![FieldWord::ZERO];
    let rows = vec![[FieldWord::ZERO; 3]];
    let attribution = vec![true];
    let (refused, peak) =
        with_peak(|| HolderSegment::new(holder, commitments, rows, u32::MAX, attribution));
    assert_eq!(
        refused,
        Err(BindingError::RowCapacityTooLarge {
            holder: HolderId::new("alice"),
            capacity: u32::MAX,
            max: MAX_HOLDER_ROW_CAPACITY,
        }),
    );
    assert!(
        peak < WORD_BYTES * 128,
        "capacity refusal allocated {peak} bytes"
    );

    let segment = HolderSegment::new(
        HolderId::new("alice"),
        vec![FieldWord::ZERO],
        vec![],
        MAX_HOLDER_ROW_CAPACITY,
        vec![true],
    )
    .unwrap();
    let statement = FederatedStatement::new(
        VerifierChallenge::from_word(FieldWord::ZERO),
        FieldWord::ZERO,
        FieldWord::ZERO,
        ValidityWindow::new(0, 1).unwrap(),
        vec![segment],
    )
    .unwrap();
    let expected = statement.public_input_word_count() * WORD_BYTES;
    assert_eq!(
        expected,
        (5 + 4 + 1 + 3 * MAX_HOLDER_ROW_CAPACITY as usize + 1) * WORD_BYTES,
    );

    for proof in [&[][..], &[0][..], &[0; WORD_BYTES][..]] {
        let (refused, peak) = with_peak(|| statement.bind_public_inputs(proof));
        assert_eq!(
            refused,
            Err(BindingError::PublicInputLength {
                expected,
                actual: proof.len()
            }),
        );
        assert!(
            peak < expected / 1024,
            "length refusal allocated {peak} bytes for a {expected}-byte layout",
        );
    }

    // Positive control: the allocator must see the full buffer when explicitly
    // reconstructing this legal statement, and the emitted bytes still bind.
    let (bytes, peak) = with_peak(|| statement.reconstruct_public_inputs());
    assert_eq!(bytes.len(), expected);
    assert!(peak >= expected, "allocator must observe reconstruction");
    statement.bind_public_inputs(&bytes).unwrap();
}
