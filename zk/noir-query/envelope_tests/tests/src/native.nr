//! Native test helper only. Generated envelope entry points cannot import this package.
use compose_core::hashes::h2;
/// Sort unsigned canonical field bytes, collapse duplicates, pad with zero to
/// the least power of two, and reduce with h2. Empty sets and zero members refuse.
/// Sorting and hashes dominate; capacity never changes the minimal tree root.
pub fn canonical_set_root<let N: u32>(members: [Field; N], count: u32) -> (Field, u32, u32) {
    std::static_assert((N > 0) & ((N & (N - 1)) == 0), "set capacity power of two");
    assert((count > 0) & (count <= N), "set count");
    let mut sorted = members;
    for i in 0..N {
        if i < count {
            assert(sorted[i] != 0, "zero set member");
        } else {
            assert(sorted[i] == 0, "set input padding");
        }
    }
    for _ in 0..N {
        for i in 0..N - 1 {
            if (i + 1 < count) & envelope_core::term::field_lt(sorted[i + 1], sorted[i]) {
                let value = sorted[i];
                sorted[i] = sorted[i + 1];
                sorted[i + 1] = value;
            }
        }
    }
    let mut row = [0; N];
    let mut unique: u32 = 0;
    for i in 0..N {
        if (i < count) & ((i == 0) | (sorted[i] != sorted[if i > 0 { i - 1 } else { 0 }])) {
            row[unique] = sorted[i];
            unique = unique + 1;
        }
    }
    let mut root = row[0];
    let mut width = N;
    let mut covered: u32 = 1;
    let mut depth: u32 = 0;
    let mut selected_depth: u32 = 0;
    for _ in 0..N {
        if width > 1 {
            for i in 0..N / 2 {
                if i < width / 2 {
                    row[i] = h2(row[2 * i], row[2 * i + 1]);
                }
            }
            width = width / 2;
            covered = covered * 2;
            depth = depth + 1;
            if (unique > covered / 2) & (unique <= covered) {
                root = row[0];
                selected_depth = depth;
            }
        }
    }
    (root, selected_depth, unique)
}
