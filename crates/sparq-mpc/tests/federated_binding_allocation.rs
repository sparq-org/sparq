//! Allocation guards for refusals and maximum-commitment statements. One test in
//! its own binary keeps the process-wide counters isolated from other tests.

#![deny(clippy::undocumented_unsafe_blocks)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use sparq_mpc::federated_binding::{
    check_freshness, BindingError, FederatedStatement, FieldWord, HolderSegment,
    InMemorySeenChallenges, ValidityWindow, VerifierChallenge, MAX_HOLDER_ROW_CAPACITY,
    MAX_PUBLIC_INPUT_BYTES, WORD_BYTES,
};
use sparq_mpc::partial::HolderId;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// Reserve one 4 KiB page (128 public-input words) for fixed-size harness and
// freshness-set bookkeeping, independent of the commitment count. Caller-owned
// statements and proof inputs are prepared before each measurement.
const ALLOCATION_SLACK_BYTES: usize = 128 * WORD_BYTES;

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
    // Prepare owned arguments before measuring. The Err assertion pins capacity
    // refusal: new() never pads, so the peak check is only an allocation sanity
    // bound, even with a disclosed row present.
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

    statement_size_refusal_does_not_allocate_sort_scratch();
    maximum_commitment_statement_stays_within_buffer_bound();
}

fn statement_size_refusal_does_not_allocate_sort_scratch() {
    // Prepare enough holders to require heap scratch with a stable sort. The
    // permutation prevents an already-sorted input from hiding the allocation.
    let holders = (0..1024)
        .map(|i| {
            HolderSegment::new(
                HolderId::new(format!("holder-{:04}", (i * 37) % 1024)),
                vec![FieldWord::ZERO],
                vec![],
                MAX_HOLDER_ROW_CAPACITY,
                vec![true],
            )
            .unwrap()
        })
        .collect();
    let window = ValidityWindow::new(0, 1).unwrap();
    let (refused, peak) = with_peak(|| {
        FederatedStatement::new(
            VerifierChallenge::from_word(FieldWord::ZERO),
            FieldWord::ZERO,
            FieldWord::ZERO,
            window,
            holders,
        )
    });
    assert_eq!(
        refused,
        Err(BindingError::PublicInputsTooLarge {
            max_bytes: MAX_PUBLIC_INPUT_BYTES,
        }),
    );
    assert!(
        peak < ALLOCATION_SLACK_BYTES,
        "statement size refusal allocated {peak} bytes of sorting scratch",
    );
}

fn maximum_commitment_statement_stays_within_buffer_bound() {
    // One holder, no rows: 5 header + 4 holder + 2 words per commitment.
    // The ceiling has an even word count, leaving one word unused.
    let commitment_count = (MAX_PUBLIC_INPUT_BYTES / WORD_BYTES - 9) / 2;
    let segment = HolderSegment::new(
        HolderId::new("alice"),
        vec![FieldWord::ZERO; commitment_count],
        vec![],
        0,
        vec![true; commitment_count],
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
    let expected_bytes = statement.public_input_word_count() * WORD_BYTES;
    assert_eq!(expected_bytes, MAX_PUBLIC_INPUT_BYTES - WORD_BYTES);
    assert!(expected_bytes + 2 * WORD_BYTES > MAX_PUBLIC_INPUT_BYTES);

    let (bytes, reconstruct_peak) = with_peak(|| statement.reconstruct_public_inputs());
    assert_eq!(bytes.len(), expected_bytes);
    assert!(
        reconstruct_peak >= expected_bytes,
        "allocator must observe the buffer"
    );
    let (bound, bind_peak) = with_peak(|| statement.bind_public_inputs(&bytes));
    bound.unwrap();
    assert!(
        bind_peak >= expected_bytes,
        "binding reconstructs the expected buffer"
    );
    for (name, peak) in [("reconstruct", reconstruct_peak), ("bind", bind_peak)] {
        assert!(
            peak <= MAX_PUBLIC_INPUT_BYTES + ALLOCATION_SLACK_BYTES,
            "{name} peak {peak} exceeds buffer cap {MAX_PUBLIC_INPUT_BYTES} + slack {ALLOCATION_SLACK_BYTES}",
        );
    }

    // The standalone digest and freshness paths allocate exactly one word per
    // part: holder_count + holder_tag + commitment_count + each commitment.
    // Pin this tighter scratch bound as well as the shared buffer envelope.
    let digest_bytes = (3 + commitment_count) * WORD_BYTES;
    let (digest, digest_peak) = with_peak(|| statement.commitment_digest());
    let (freshness, freshness_peak) = with_peak(|| statement.freshness_binding());
    assert_eq!(freshness.commitment_digest, digest);
    let mut seen = InMemorySeenChallenges::default();
    let (checked, check_peak) =
        with_peak(|| check_freshness(&statement, &statement.challenge(), 0, &mut seen));
    assert_eq!(checked.unwrap(), freshness.tag());
    for (name, peak) in [
        ("digest", digest_peak),
        ("freshness", freshness_peak),
        ("check_freshness", check_peak),
    ] {
        assert!(peak <= MAX_PUBLIC_INPUT_BYTES + ALLOCATION_SLACK_BYTES);
        assert!(
            peak <= digest_bytes + ALLOCATION_SLACK_BYTES,
            "{name} peak {peak} exceeds digest parts {digest_bytes} + slack {ALLOCATION_SLACK_BYTES}",
        );
    }
    eprintln!(
        "maximum commitments={commitment_count}; buffer cap={MAX_PUBLIC_INPUT_BYTES}; slack={ALLOCATION_SLACK_BYTES}; reconstruct peak={reconstruct_peak}; bind peak={bind_peak}; digest peak={digest_peak}; freshness peak={freshness_peak}; check_freshness peak={check_peak}",
    );
}
