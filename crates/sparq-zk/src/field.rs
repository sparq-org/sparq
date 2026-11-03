//! BN254 scalar-field helpers.
//!
//! `Fr` (ark-bn254's scalar field) is exactly Noir's `Field` type for the
//! default BN254 backend, so every value produced here can be handed to a
//! Noir circuit witness verbatim.

use ark_ff::{BigInteger, PrimeField};

pub type Fr = ark_bn254::Fr;

/// Decodes a big-endian hex string (no `0x` prefix) and reduces it into the
/// field. Mirrors noir's `field_from_hex` (`from_be_bytes_reduce`).
pub(crate) fn field_from_hex(hex: &str) -> Fr {
    let bytes = decode_hex(hex);
    Fr::from_be_bytes_mod_order(&bytes)
}

/// Canonical lowercase hex (64 nibbles, `0x` prefixed) of a field element —
/// the representation stored in `<urn:sparq:zk>` registry literals and
/// compared against `nargo` output in the cross-tests.
pub fn field_to_hex(f: &Fr) -> String {
    let bytes = f.into_bigint().to_bytes_be();
    let mut s = String::with_capacity(2 + 64);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Parses a `0x`-prefixed (or bare) hex field literal, accepting fewer than
/// 64 nibbles (left-padded), reducing mod p.
pub fn field_from_hex_str(s: &str) -> Option<Fr> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.is_empty() || s.len() > 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let padded = format!("{s:0>64}");
    Some(field_from_hex(&padded))
}

/// Parses one to 64 hex nibbles, with an optional `0x` prefix, in either case.
/// Returns `None` for malformed text or an integer at or above the modulus.
/// Unlike [`field_from_hex_str`], this parser never reduces its input.
pub fn field_from_hex_str_strict(s: &str) -> Option<Fr> {
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    if s.is_empty() || s.len() > 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = decode_hex(&format!("{s:0>64}"));
    let mut integer = <Fr as PrimeField>::BigInt::default();
    for (limb, chunk) in integer.as_mut().iter_mut().rev().zip(bytes.chunks_exact(8)) {
        *limb = u64::from_be_bytes(chunk.try_into().ok()?);
    }
    if integer >= Fr::MODULUS {
        return None;
    }
    Fr::from_bigint(integer)
}

/// 32-byte big-endian (left-zero-padded) representation of a field element —
/// exactly one word of bb's `public_inputs` blob layout (each public input is a
/// 32-byte BE field element, no length prefix; determined empirically against
/// bb 5.0.0-nightly, see the `sparq-zk-compose` verifier reconstruction). The
/// hex of this word is `field_to_hex` minus the `0x`.
// added for the verifier's public-input reconstruction (audit #1).
pub fn field_to_be_bytes_32(f: &Fr) -> [u8; 32] {
    let be = f.into_bigint().to_bytes_be();
    // ark `to_bytes_be` for a 254-bit field returns up to 32 bytes; left-pad to
    // a fixed 32-byte word so a small value (e.g. `op = 0`) still emits a full
    // word, matching bb's fixed-width public-input serialization.
    let mut out = [0u8; 32];
    debug_assert!(be.len() <= 32, "bn254 field is <= 32 bytes");
    let start = 32 - be.len();
    out[start..].copy_from_slice(&be);
    out
}

/// Maps 32 hash bytes (e.g. a Blake3 digest) into the field by truncating to
/// the low 31 bytes (248 bits < 254-bit modulus) interpreted big-endian.
/// Truncation rather than modular reduction keeps the encoding bias-free and
/// trivially recomputable in-circuit (the value is a witness; the circuit
/// never re-derives it from bytes — see plan §2.2's off-circuit `h_s`).
pub fn field_from_hash_bytes(bytes: &[u8; 32]) -> Fr {
    Fr::from_be_bytes_mod_order(&bytes[1..])
}

fn decode_hex(hex: &str) -> Vec<u8> {
    // sq-hbg7: stable-1.96 clippy `manual_is_multiple_of`.
    assert!(hex.len().is_multiple_of(2), "hex string must have even length");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_hex_parser_checks_range_and_spellings() {
        let max = "30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000000";
        let modulus = "30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001";
        let above = "30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000002";
        assert_eq!(field_from_hex_str_strict(max), Some(-Fr::from(1u64)));
        assert_eq!(field_from_hex_str_strict(modulus), None);
        assert_eq!(field_from_hex_str_strict(above), None);
        for text in ["aBc", "0x0ABC", "0Xabc"] {
            assert_eq!(field_from_hex_str_strict(text), Some(Fr::from(0xabcu64)));
        }
        assert_eq!(field_from_hex_str_strict("0"), Some(Fr::from(0u64)));
        for text in ["", "0x", "0xz", " 1", "1\n", "é", &"0".repeat(65)] {
            assert_eq!(field_from_hex_str_strict(text), None, "{text:?}");
        }
        // The separate reducing parser retains its existing contract.
        assert_eq!(field_from_hex_str(modulus), Some(Fr::from(0u64)));
    }

    #[test]
    fn hex_round_trip() {
        let f = field_from_hex_str("0x1a2b").unwrap();
        assert_eq!(f, Fr::from(0x1a2bu64));
        let h = field_to_hex(&f);
        assert_eq!(h, format!("0x{:0>64}", "1a2b"));
        assert_eq!(field_from_hex_str(&h).unwrap(), f);
    }

    #[test]
    fn be_bytes_32_is_left_padded_and_matches_hex() {
        // A small value emits a full 32-byte word (left-padded), and its hex
        // equals field_to_hex minus the 0x — the bb public-input word layout.
        let f = Fr::from(0x2au64);
        let w = field_to_be_bytes_32(&f);
        assert_eq!(w.len(), 32);
        assert_eq!(w[31], 0x2a);
        assert!(w[..31].iter().all(|&b| b == 0));
        let hex: String = w.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(format!("0x{hex}"), field_to_hex(&f));
        // Zero is the all-zero word (variable slots / padding rows).
        assert_eq!(field_to_be_bytes_32(&Fr::from(0u64)), [0u8; 32]);
    }

    #[test]
    fn hash_bytes_truncate_to_248_bits() {
        let bytes = [0xffu8; 32];
        let f = field_from_hash_bytes(&bytes);
        // 2^248 - 1 < p, so the value must round-trip exactly.
        let expected = field_from_hex_str(&"ff".repeat(31)).unwrap();
        assert_eq!(f, expected);
    }

    #[test]
    fn rejects_garbage() {
        assert!(field_from_hex_str("0xzz").is_none());
        assert!(field_from_hex_str("").is_none());
        assert!(field_from_hex_str(&"f".repeat(65)).is_none());
    }
}
