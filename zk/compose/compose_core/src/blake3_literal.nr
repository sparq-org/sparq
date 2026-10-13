//! Variable-length BLAKE3 opening of a canonical xsd:integer literal.
//! Research-stage, not externally audited.
//!
//! One hash over two compressions covers every token length (47 to 66 bytes),
//! so the circuit neither selects nor reveals the digit count. The rotations
//! and 32-bit additions are written as field constraints, and the token is
//! built as weighted sums of selector-times-digit products rather than
//! per-length conditional writes. `proof-methods/acir/` has the derivation
//! and a hand-written ACIR version of the same relation.

global IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];
global PERMUTATION: [u32; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
global CHUNK_START: Field = 1;
global CHUNK_END: Field = 2;
global ROOT: Field = 8;

unconstrained fn split(x: u32, r: u32) -> (Field, Field) {
    ((x & ((1 << r) - 1)) as Field, ((x >> r) | (x << (32 - r))) as Field)
}

/// x >>> R: x = rot * 2^R - lo * (2^32 - 1) with lo < 2^R and rot < 2^32.
fn rotr<let R: u32>(x: u32) -> u32 {
    // Safety: lo and rot are bounded and tied to x below.
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

/// (sum of at most three 32-bit terms) mod 2^32, with an explicit carry.
fn add(s: Field) -> u32 {
    // Safety: out and the carry are bounded and tied to s below.
    let (out, carry) = unsafe { carry_split(s) };
    out.assert_max_bit_size::<32>();
    carry.assert_max_bit_size::<2>();
    assert_eq(s, out + carry * 4294967296);
    out as u32
}

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

/// The BLAKE3 compression function, returning the chaining value.
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

/// Twenty decimal digits of value (most significant first) and a one-hot
/// selector for the lexical length (sel[k] = 1 iff the token has k + 1
/// characters between its quotes, sign included).
unconstrained fn lexical(magnitude: u64, negative: bool) -> ([Field; 20], [Field; 20]) {
    let mut digits: [Field; 20] = [0; 20];
    let mut v = magnitude;
    for i in 0..20 {
        digits[19 - i] = (v % 10) as Field;
        v /= 10;
    }
    let mut count: u32 = 1;
    let mut w = magnitude / 10;
    while w != 0 {
        count += 1;
        w /= 10;
    }
    let mut sel: [Field; 20] = [0; 20];
    sel[count + negative as u32 - 1] = 1;
    (digits, sel)
}

unconstrained fn to_u32(x: Field) -> u32 {
    x as u32
}

unconstrained fn to_bytes(x: u32) -> [u8; 4] {
    [x as u8, (x >> 8) as u8, (x >> 16) as u8, (x >> 24) as u8]
}

/// A field element below 2^32 by construction, as a u32.
fn word(x: Field) -> u32 {
    // Safety: the result is a range-checked u32 equal to x.
    let w = unsafe { to_u32(x) };
    assert_eq(w as Field, x);
    w
}

/// big-endian(blake3(token)[1..32]) for the canonical xsd:integer token
/// `"[-]digits"^^<http://www.w3.org/2001/XMLSchema#integer>` of the integer
/// with this magnitude and sign. The caller ensures a negative value has a
/// nonzero magnitude.
pub fn integer_literal_hs(magnitude: u64, negative: bool) -> Field {
    let neg = negative as Field;
    // Safety: digits are bounded to 0..9 and recombine to the magnitude; sel
    // is boolean, one-hot and consistent with the digits, all checked below.
    let (digits, sel) = unsafe { lexical(magnitude, negative) };
    let mut sum: Field = 0;
    for i in 0..20 {
        digits[i].assert_max_bit_size::<4>();
        (9 - digits[i]).assert_max_bit_size::<4>();
        sum = sum * 10 + digits[i];
    }
    assert_eq(sum, magnitude as Field);
    let mut one: Field = 0;
    for k in 0..20 {
        assert_eq(sel[k] * sel[k], sel[k]);
        one += sel[k];
    }
    assert_eq(one, 1);
    // Every slot left of the token is zero.
    for k in 1..20 {
        let mut at_least: Field = 0;
        for j in k..20 {
            at_least += sel[j];
        }
        assert_eq(digits[19 - k] * (1 - at_least), 0);
    }
    // The token's first slot holds the sign when negative (so its digit is
    // zero), and a lone sign is not a token.
    let mut top: Field = 0;
    let mut next: Field = 0;
    for k in 0..20 {
        top += sel[k] * digits[19 - k];
        if k > 0 {
            next += sel[k] * digits[20 - k];
        }
    }
    assert_eq(top * neg, 0);
    assert_eq(sel[0] * neg, 0);
    // Canonical form: the leading digit is nonzero unless the magnitude has
    // one digit.
    let lead = top + neg * next;
    let one_digit = sel[0] + neg * sel[1];
    (lead - 1 + one_digit).assert_max_bit_size::<4>();

    let suffix: [u8; 45] = "\"^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    let mut words: [u32; 32] = [0; 32];
    for w in 0..17 {
        let mut e: Field = 0;
        for k in 0..20 {
            let len = k + 1;
            for q in 0..4 {
                let p = 4 * w + q;
                let scale: Field = (1 << (8 * q)) as Field;
                if p == 0 {
                    e += sel[k] * 34 * scale;
                } else if p <= len {
                    let mut c = sel[k] * digits[20 - len + p - 1] + sel[k] * 48;
                    if p == 1 {
                        // '-' is 45, three below '0'.
                        c -= sel[k] * neg * 3;
                    }
                    e += c * scale;
                } else if p < len + 46 {
                    e += sel[k] * (suffix[p - len - 1] as Field) * scale;
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
    // Tokens of 65 or 66 bytes take a second block.
    let two = sel[18] + sel[19];
    let mut len1: Field = two * 64;
    for k in 0..18 {
        len1 += sel[k] * ((k + 47) as Field);
    }
    let flags1 = CHUNK_START + CHUNK_END + ROOT - two * (CHUNK_END + ROOT);
    let out1 = compress(IV, first, word(len1), word(flags1));
    let out2 = compress(
        out1,
        second,
        word(sel[18] + sel[19] * 2),
        word(CHUNK_END + ROOT),
    );
    let mut hs: Field = 0;
    for i in 0..8 {
        let o1 = out1[i] as Field;
        let digest = o1 + two * (out2[i] as Field - o1);
        // Safety: the bytes are range-checked u8s that recombine to the word.
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

fn std_hs<let L: u32>(lexical: [u8; L]) -> Field {
    let suffix = "\"^^<http://www.w3.org/2001/XMLSchema#integer>".as_bytes();
    let mut token: [u8; L + 46] = [0; L + 46];
    token[0] = 34;
    for i in 0..L {
        token[i + 1] = lexical[i];
    }
    for i in 0..45 {
        token[L + 1 + i] = suffix[i];
    }
    let digest = std::hash::blake3(token);
    let mut hs: Field = 0;
    for i in 1..32 {
        hs = hs * 256 + digest[i] as Field;
    }
    hs
}

#[test]
fn matches_std_blake3_at_every_length_boundary() {
    assert_eq(integer_literal_hs(0, false), std_hs("0".as_bytes()));
    assert_eq(integer_literal_hs(9, false), std_hs("9".as_bytes()));
    assert_eq(integer_literal_hs(10, false), std_hs("10".as_bytes()));
    assert_eq(
        integer_literal_hs(999999999999999999, false),
        std_hs("999999999999999999".as_bytes()),
    );
    assert_eq(
        integer_literal_hs(1000000000000000000, false),
        std_hs("1000000000000000000".as_bytes()),
    );
    assert_eq(
        integer_literal_hs(9999999999999999999, false),
        std_hs("9999999999999999999".as_bytes()),
    );
    assert_eq(
        integer_literal_hs(10000000000000000000, false),
        std_hs("10000000000000000000".as_bytes()),
    );
    assert_eq(
        integer_literal_hs(18446744073709551615, false),
        std_hs("18446744073709551615".as_bytes()),
    );
    assert_eq(integer_literal_hs(1, true), std_hs("-1".as_bytes()));
    assert_eq(integer_literal_hs(10, true), std_hs("-10".as_bytes()));
    assert_eq(integer_literal_hs(99999999999999999, true), std_hs("-99999999999999999".as_bytes()));
    assert_eq(
        integer_literal_hs(100000000000000000, true),
        std_hs("-100000000000000000".as_bytes()),
    );
    assert_eq(
        integer_literal_hs(9223372036854775808, true),
        std_hs("-9223372036854775808".as_bytes()),
    );
}
