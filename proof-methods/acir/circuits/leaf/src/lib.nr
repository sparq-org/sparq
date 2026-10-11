// Shared parts of the opening prototypes: the comparison verdict and a
// Noir version of the variable-length BLAKE3 opening in ../../blake3_leaf.py.

use std::ops::WrappingAdd;

/// Openings per circuit. Eight is R x F of result_v2_k2_n16_p3_r4_f2_i64_d20.
pub global N: u32 = 8;

global IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
global PERMUTATION: [u32; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

/// The comparison of filter_int_check: 0=lt 1=le 2=gt 3=ge 4=eq 5=ne.
pub fn verdict(op: u32, bound: u64, value: u64) -> bool {
    assert(op <= 5, "unknown comparison operator");
    let lt = value < bound;
    let eq = value == bound;
    if op == 0 {
        lt
    } else if op == 1 {
        lt | eq
    } else if op == 2 {
        !lt & !eq
    } else if op == 3 {
        !lt
    } else if op == 4 {
        eq
    } else {
        !eq
    }
}

fn rotr(x: u32, r: u32) -> u32 {
    (x >> r) | (x << (32 - r))
}

fn g(s: &mut [u32; 16], a: u32, b: u32, c: u32, d: u32, x: u32, y: u32) {
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(x);
    s[d] = rotr(s[d] ^ s[a], 16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = rotr(s[b] ^ s[c], 12);
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(y);
    s[d] = rotr(s[d] ^ s[a], 8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = rotr(s[b] ^ s[c], 7);
}

fn compress(cv: [u32; 8], block: [u32; 16], block_len: u32, flags: u32) -> [u32; 8] {
    let mut s: [u32; 16] = [0; 16];
    for i in 0..8 {
        s[i] = cv[i];
    }
    for i in 0..4 {
        s[8 + i] = IV[i];
    }
    s[14] = block_len;
    s[15] = flags;
    let mut m = block;
    for _ in 0..7 {
        g(&mut s, 0, 4, 8, 12, m[0], m[1]);
        g(&mut s, 1, 5, 9, 13, m[2], m[3]);
        g(&mut s, 2, 6, 10, 14, m[4], m[5]);
        g(&mut s, 3, 7, 11, 15, m[6], m[7]);
        g(&mut s, 0, 5, 10, 15, m[8], m[9]);
        g(&mut s, 1, 6, 11, 12, m[10], m[11]);
        g(&mut s, 2, 7, 8, 13, m[12], m[13]);
        g(&mut s, 3, 4, 9, 14, m[14], m[15]);
        let mut next = m;
        for i in 0..16 {
            next[i] = m[PERMUTATION[i]];
        }
        m = next;
    }
    let mut out: [u32; 8] = [0; 8];
    for i in 0..8 {
        out[i] = s[i] ^ s[i + 8];
    }
    out
}

unconstrained fn digit_count(value: u64) -> u32 {
    let mut n = 1;
    let mut v = value / 10;
    while v != 0 {
        n += 1;
        v /= 10;
    }
    n
}

/// big-endian(blake3(token)[1..32]) for the canonical xsd:integer token of
/// value, with one variable-length hash over two compressions.
pub fn literal_hash(value: u64) -> Field {
    let mut digits: [u8; 20] = [0; 20];
    let mut remaining = value;
    for i in 0..20 {
        digits[19 - i] = (remaining % 10) as u8;
        remaining = remaining / 10;
    }
    // Safety: the selector is checked against the digits below.
    let n = unsafe { digit_count(value) };
    assert((n >= 1) & (n <= 20), "digit count");
    let mut sel: [bool; 20] = [false; 20];
    for k in 0..20 {
        sel[k] = n == k + 1;
    }
    // Every digit above the count is zero; the leading digit is nonzero
    // unless the value has one digit.
    for k in 1..20 {
        if n <= k {
            assert(digits[19 - k] == 0, "digit above the count");
        }
    }
    if n > 1 {
        assert(digits[20 - n] != 0, "non-canonical leading zero");
    }

    let suffix: [u8; 45] = "\"^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    let mut bytes: [u8; 128] = [0; 128];
    for k in 0..20 {
        if sel[k] {
            let d = k + 1;
            bytes[0] = 34;
            for p in 1..21 {
                if p <= d {
                    bytes[p] = 48 + digits[20 - d + p - 1];
                }
            }
            for i in 0..45 {
                bytes[d + 1 + i] = suffix[i];
            }
        }
    }
    let mut words: [u32; 32] = [0; 32];
    for w in 0..32 {
        words[w] = (bytes[4 * w] as u32)
            + ((bytes[4 * w + 1] as u32) << 8)
            + ((bytes[4 * w + 2] as u32) << 16)
            + ((bytes[4 * w + 3] as u32) << 24);
    }
    let mut first: [u32; 16] = [0; 16];
    let mut second: [u32; 16] = [0; 16];
    for i in 0..16 {
        first[i] = words[i];
        second[i] = words[16 + i];
    }
    let two = n > 18;
    let len = n + 46;
    let (len1, flags1) = if two { (64, 1) } else { (len, 11) };
    let out1 = compress(IV, first, len1, flags1);
    let out2 = compress(out1, second, if two { len - 64 } else { 0 }, 10);
    let digest = if two { out2 } else { out1 };
    let mut hs: Field = 0;
    for i in 0..8 {
        for q in 0..4 {
            let byte = (digest[i] >> (8 * q)) & 0xFF;
            if (i, q) != (0, 0) {
                hs = hs * 256 + byte as Field;
            }
        }
    }
    hs
}
