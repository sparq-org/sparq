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

unconstrained fn split(x: u32, r: u32) -> (Field, Field) {
    ((x & ((1 << r) - 1)) as Field, ((x >> r) | (x << (32 - r))) as Field)
}

/// x >>> R as the hand-written ACIR does it: x = rot * 2^R - lo * (2^32 - 1)
/// with lo < 2^R and rot < 2^32. Both bounds are explicit; the compiler drops
/// the 32-bit check where an XOR on the result already implies it.
fn rotr<let R: u32>(x: u32) -> u32 {
    // Safety: lo and rot are constrained below.
    let (lo, rot) = unsafe { split(x, R) };
    lo.assert_max_bit_size::<R>();
    rot.assert_max_bit_size::<32>();
    let p: Field = (1 << R) as Field;
    assert_eq(x as Field, rot * p - lo * 4294967295);
    rot as u32
}

unconstrained fn carry_split(s: Field) -> (Field, Field) {
    let v = s as u64;
    ((v & 0xFFFFFFFF) as Field, (v >> 32) as Field)
}

/// (sum of 32-bit terms) mod 2^32 with an explicit carry of at most 2 bits.
fn add(s: Field) -> u32 {
    // Safety: out and the carry are range-checked below.
    let (out, carry) = unsafe { carry_split(s) };
    out.assert_max_bit_size::<32>();
    carry.assert_max_bit_size::<2>();
    assert_eq(s, out + carry * 4294967296);
    out as u32
}

/// The obvious Noir: u32 shifts, OR and wrapping_add.
fn g_plain(s: &mut [u32; 16], a: u32, b: u32, c: u32, d: u32, x: u32, y: u32) {
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(x);
    s[d] = rotr_plain(s[d] ^ s[a], 16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = rotr_plain(s[b] ^ s[c], 12);
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(y);
    s[d] = rotr_plain(s[d] ^ s[a], 8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = rotr_plain(s[b] ^ s[c], 7);
}

fn rotr_plain(x: u32, r: u32) -> u32 {
    (x >> r) | (x << (32 - r))
}

/// The same G with the rotations and additions written as field
/// constraints, the way blake3_leaf.py emits them.
fn g(s: &mut [u32; 16], a: u32, b: u32, c: u32, d: u32, x: u32, y: u32) {
    s[a] = add(s[a] as Field + s[b] as Field + x as Field);
    s[d] = rotr::<16>(s[d] ^ s[a]);
    s[c] = add(s[c] as Field + s[d] as Field);
    s[b] = rotr::<12>(s[b] ^ s[c]);
    s[a] = add(s[a] as Field + s[b] as Field + y as Field);
    s[d] = rotr::<8>(s[d] ^ s[a]);
    s[c] = add(s[c] as Field + s[d] as Field);
    s[b] = rotr::<7>(s[b] ^ s[c]);
}

fn compress<let TUNED: u32>(
    cv: [u32; 8],
    block: [u32; 16],
    block_len: u32,
    flags: u32,
) -> [u32; 8] {
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
        if TUNED == 1 {
            g(&mut s, 0, 4, 8, 12, m[0], m[1]);
        } else {
            g_plain(&mut s, 0, 4, 8, 12, m[0], m[1]);
        }
        if TUNED == 1 {
            g(&mut s, 1, 5, 9, 13, m[2], m[3]);
        } else {
            g_plain(&mut s, 1, 5, 9, 13, m[2], m[3]);
        }
        if TUNED == 1 {
            g(&mut s, 2, 6, 10, 14, m[4], m[5]);
        } else {
            g_plain(&mut s, 2, 6, 10, 14, m[4], m[5]);
        }
        if TUNED == 1 {
            g(&mut s, 3, 7, 11, 15, m[6], m[7]);
        } else {
            g_plain(&mut s, 3, 7, 11, 15, m[6], m[7]);
        }
        if TUNED == 1 {
            g(&mut s, 0, 5, 10, 15, m[8], m[9]);
        } else {
            g_plain(&mut s, 0, 5, 10, 15, m[8], m[9]);
        }
        if TUNED == 1 {
            g(&mut s, 1, 6, 11, 12, m[10], m[11]);
        } else {
            g_plain(&mut s, 1, 6, 11, 12, m[10], m[11]);
        }
        if TUNED == 1 {
            g(&mut s, 2, 7, 8, 13, m[12], m[13]);
        } else {
            g_plain(&mut s, 2, 7, 8, 13, m[12], m[13]);
        }
        if TUNED == 1 {
            g(&mut s, 3, 4, 9, 14, m[14], m[15]);
        } else {
            g_plain(&mut s, 3, 4, 9, 14, m[14], m[15]);
        }
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
/// value, with one variable-length hash over two compressions. TUNED = 1
/// writes the rotations, additions and token as field constraints; 0 uses
/// u32 ops and conditional writes.
pub fn literal_hash<let TUNED: u32>(value: u64) -> Field {
    if TUNED == 1 {
        literal_hash_tuned(value)
    } else {
        literal_hash_plain(value)
    }
}

unconstrained fn decimal(value: u64) -> ([Field; 20], [Field; 20]) {
    let mut digits: [Field; 20] = [0; 20];
    let mut v = value;
    for i in 0..20 {
        digits[19 - i] = (v % 10) as Field;
        v /= 10;
    }
    let mut sel: [Field; 20] = [0; 20];
    sel[digit_count(value) - 1] = 1;
    (digits, sel)
}

unconstrained fn to_u32(x: Field) -> u32 {
    x as u32
}

unconstrained fn to_bytes(x: u32) -> [u8; 4] {
    [x as u8, (x >> 8) as u8, (x >> 16) as u8, (x >> 24) as u8]
}

/// A field element known to be below 2^32 by construction, as a u32.
fn word(x: Field) -> u32 {
    // Safety: the result is a u32 (range-checked) equal to x.
    let w = unsafe { to_u32(x) };
    assert_eq(w as Field, x);
    w
}

/// literal_hash<1> with the token built the way blake3_leaf.py builds it:
/// digits and a one-hot digit count as hinted field elements, and each
/// message word as one weighted sum of selector-times-digit products, so
/// there is no per-length conditional write.
fn literal_hash_tuned(value: u64) -> Field {
    // Safety: digits are bounded to 0..9 and recombine to value; sel is
    // boolean, sums to one and matches the digits, all checked below.
    let (digits, sel) = unsafe { decimal(value) };
    let mut sum: Field = 0;
    for i in 0..20 {
        digits[i].assert_max_bit_size::<4>();
        (9 - digits[i]).assert_max_bit_size::<4>();
        sum = sum * 10 + digits[i];
    }
    assert_eq(sum, value as Field);
    let mut one: Field = 0;
    for k in 0..20 {
        assert_eq(sel[k] * sel[k], sel[k]);
        one += sel[k];
    }
    assert_eq(one, 1);
    // Every digit above the count is zero.
    for k in 1..20 {
        let mut at_least: Field = 0;
        for j in k..20 {
            at_least += sel[j];
        }
        assert_eq(digits[19 - k] * (1 - at_least), 0);
    }
    // Canonical form: the leading digit is nonzero unless the value is 0..9.
    let mut leading: Field = 0;
    for k in 0..20 {
        leading += sel[k] * digits[19 - k];
    }
    (leading - 1 + sel[0]).assert_max_bit_size::<4>();

    let suffix: [u8; 45] = "\"^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    let mut words: [u32; 32] = [0; 32];
    for w in 0..17 {
        let mut e: Field = 0;
        for k in 0..20 {
            let d = k + 1;
            for q in 0..4 {
                let p = 4 * w + q;
                let scale: Field = (1 << (8 * q)) as Field;
                if p == 0 {
                    e += sel[k] * 34 * scale;
                } else if p <= d {
                    e += (sel[k] * digits[20 - d + p - 1] + sel[k] * 48) * scale;
                } else if p < d + 46 {
                    e += sel[k] * (suffix[p - d - 1] as Field) * scale;
                }
            }
        }
        words[w] = word(e);
    }
    let mut first: [u32; 16] = [0; 16];
    let mut second: [u32; 16] = [0; 16];
    for i in 0..16 {
        first[i] = words[i];
        second[i] = words[16 + i];
    }
    let two = sel[18] + sel[19];
    let mut len1: Field = two * 64;
    for k in 0..18 {
        len1 += sel[k] * ((k + 47) as Field);
    }
    let flags1 = 11 - two * 10;
    let out1 = compress::<1>(IV, first, word(len1), word(flags1));
    let out2 = compress::<1>(out1, second, word(sel[18] + sel[19] * 2), 10);
    let mut hs: Field = 0;
    for i in 0..8 {
        let o1 = out1[i] as Field;
        let digest = o1 + two * (out2[i] as Field - o1);
        // Safety: the bytes are u8 (range-checked) and recombine to the word.
        let b = unsafe { to_bytes(word(digest)) };
        let mut sum: Field = 0;
        for q in 0..4 {
            sum += (b[q] as Field) * ((1 << (8 * q)) as Field);
            if (i, q) != (0, 0) {
                hs = hs * 256 + b[q] as Field;
            }
        }
        assert_eq(sum, digest);
    }
    hs
}

fn literal_hash_plain(value: u64) -> Field {
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
    let out1 = compress::<0>(IV, first, len1, flags1);
    let out2 = compress::<0>(out1, second, if two { len - 64 } else { 0 }, 10);
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
