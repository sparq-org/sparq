use std::cell::Cell;

thread_local! {
    static STATE: Cell<u64> = Cell::new(u64::from_le_bytes(
        uuid::Uuid::new_v4().as_bytes()[..8].try_into().expect("8 bytes"),
    ));
}

/// The next value in `[0, 1)` — splitmix64 output through the same 53-bit
/// mantissa construction the old per-row OS draw used.
pub(super) fn next() -> f64 {
    STATE.with(|s| {
        let seed = s.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        s.set(seed);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    })
}
