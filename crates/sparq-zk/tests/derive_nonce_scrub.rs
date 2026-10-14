//! Deterministic signing never frees a heap block that still holds the secret key or the
//! secret nonce. A counting allocator scans every block freed while signing for their
//! 32-byte little- and big-endian encodings. Stack and register copies are out of reach.
#![warn(clippy::undocumented_unsafe_blocks)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;

use ark_ed_on_bn254::Fr as JjScalar;
use ark_ff::{BigInteger, PrimeField};
use sparq_zk::field::Fr;
use sparq_zk::sig::{in_circuit_witness, sign_deterministic, SecretKey};

struct ScanningAllocator;

static ARMED: AtomicBool = AtomicBool::new(false);
static HITS: AtomicUsize = AtomicUsize::new(0);
static PATTERNS: OnceLock<[[u8; 32]; 4]> = OnceLock::new();

// SAFETY: both methods forward to `System` with the caller's arguments unchanged, so `System`
// discharges `GlobalAlloc`'s obligations. Every block is allocated zeroed, so all of its bytes
// are initialised from the start (the default `realloc` goes through these two methods and
// copies into a zeroed block). `dealloc` READS the block it is handed before forwarding it;
// the block is live, initialised and `layout.size()` bytes long until `System.dealloc`
// returns, and nothing is written or retained.
unsafe impl GlobalAlloc for ScanningAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's `layout` contract is forwarded to `System.alloc_zeroed`
        // unchanged. Zeroing initialises every byte, which `dealloc`'s read relies on.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System.alloc_zeroed` (via `alloc` above, or the default
        // `alloc_zeroed`/`realloc`, which call it) with `layout`, so it points to
        // `layout.size()` live bytes that are all initialised; they are only read as `u8`
        // before the block is returned to `System` with the same arguments.
        unsafe {
            if ARMED.load(Ordering::SeqCst) {
                scan(std::slice::from_raw_parts(ptr, layout.size()));
            }
            System.dealloc(ptr, layout)
        }
    }
}

fn scan(block: &[u8]) {
    if let Some(patterns) = PATTERNS.get() {
        if patterns.iter().any(|p| block.windows(32).any(|w| w == p)) {
            HITS.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[global_allocator]
static ALLOCATOR: ScanningAllocator = ScanningAllocator;

fn encodings(x: JjScalar) -> [[u8; 32]; 2] {
    let le: [u8; 32] = x.into_bigint().to_bytes_le().try_into().unwrap();
    let mut be = le;
    be.reverse();
    [le, be]
}

#[test]
fn signing_frees_no_heap_copy_of_the_key_or_nonce() {
    // A key and message whose encodings contain no zero bytes, so a match is not chance.
    let sk = SecretKey(
        JjScalar::from(0x1f2e_3d4c_5b6a_7988u64) * JjScalar::from(0x0123_4567_89ab_cdefu64),
    );
    let m = Fr::from(0xc0ffee_u64);
    let sig = sign_deterministic(&sk, &m);
    // Recover the nonce from s = k + e·sk.
    let witness = in_circuit_witness(&sk.public_key(), &m, &sig).expect("non-degenerate key");
    let e = JjScalar::from_bigint(witness.e.into_bigint()).expect("e < L");
    let k = sig.s - e * sk.0;
    let [sk_le, sk_be] = encodings(sk.0);
    let [k_le, k_be] = encodings(k);
    PATTERNS.set([sk_le, sk_be, k_le, k_be]).unwrap();

    ARMED.store(true, Ordering::SeqCst);
    let again = sign_deterministic(&sk, &m);
    ARMED.store(false, Ordering::SeqCst);

    assert_eq!(again, sig);
    assert_eq!(
        HITS.load(Ordering::SeqCst),
        0,
        "a freed heap block held the key or the nonce"
    );
}
