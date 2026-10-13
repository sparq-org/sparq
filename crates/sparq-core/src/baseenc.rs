//! Base58btc and unpadded base64url codecs shared across the workspace.
//!
//! These are the multibase `z` and `u` bodies (no multibase prefix). Both decoders are
//! strict: a character outside the alphabet is [`DecodeError::InvalidChar`], and an
//! unpadded base64url body whose length is impossible (≡ 1 mod 4) or whose final
//! character carries non-zero unused bits is [`DecodeError::NonCanonical`], so each byte
//! string has exactly one accepted base64url text. Base58btc is canonical by construction
//! (each leading `1` is one leading zero byte).

use std::fmt;

const BASE58BTC: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Why a base58btc or base64url body failed to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// A character outside the alphabet (including `=` padding and base64's `+` / `/`).
    InvalidChar(char),
    /// An unpadded base64url body with an impossible length or non-zero unused bits.
    NonCanonical,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::InvalidChar(c) => write!(f, "character {c:?} is not in the alphabet"),
            DecodeError::NonCanonical => f.write_str("non-canonical padding bits or length"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Appends the base58btc encoding of `bytes` to `out`. Each leading zero byte becomes a
/// leading `1`; empty input appends nothing.
pub fn base58btc_encode_into(bytes: &[u8], out: &mut String) {
    let leading = bytes.iter().take_while(|&&b| b == 0).count();
    // Little-endian base-58 digits of the non-zero tail.
    let mut digits: Vec<u8> = Vec::with_capacity((bytes.len() - leading) * 138 / 100 + 1);
    for &b in &bytes[leading..] {
        let mut carry = u32::from(b);
        for d in &mut digits {
            carry += u32::from(*d) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    out.reserve(leading + digits.len());
    out.extend(std::iter::repeat_n('1', leading));
    out.extend(
        digits
            .iter()
            .rev()
            .map(|&d| BASE58BTC[usize::from(d)] as char),
    );
}

/// The base58btc encoding of `bytes`.
pub fn base58btc_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    base58btc_encode_into(bytes, &mut out);
    out
}

/// Decodes a base58btc body. Each leading `1` is one leading zero byte; the empty string
/// decodes to no bytes.
pub fn base58btc_decode(s: &str) -> Result<Vec<u8>, DecodeError> {
    let leading = s.bytes().take_while(|&c| c == b'1').count();
    // Little-endian base-256 bytes of the value after the leading '1's.
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    for c in s[leading..].chars() {
        let v = u8::try_from(c)
            .ok()
            .and_then(|b| BASE58BTC.iter().position(|&t| t == b))
            .ok_or(DecodeError::InvalidChar(c))?;
        let mut carry = v as u32;
        for b in &mut bytes {
            carry += u32::from(*b) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    bytes.extend(std::iter::repeat_n(0, leading));
    bytes.reverse();
    Ok(bytes)
}

/// Appends the unpadded base64url (RFC 4648 §5) encoding of `bytes` to `out`.
pub fn base64url_encode_into(bytes: &[u8], out: &mut String) {
    out.reserve(bytes.len().div_ceil(3) * 4);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(BASE64URL[((acc >> bits) & 0x3f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE64URL[((acc << (6 - bits)) & 0x3f) as usize] as char);
    }
}

/// The unpadded base64url encoding of `bytes`.
pub fn base64url_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    base64url_encode_into(bytes, &mut out);
    out
}

/// Decodes a strict unpadded base64url body: no `=` padding, no characters from the
/// standard base64 alphabet, and only the one canonical text per byte string.
pub fn base64url_decode(s: &str) -> Result<Vec<u8>, DecodeError> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '-' => 62,
            '_' => 63,
            _ => return Err(DecodeError::InvalidChar(c)),
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    // A whole unused sextet means length ≡ 1 (mod 4); set unused bits mean a second text.
    if bits >= 6 || acc & ((1u32 << bits) - 1) != 0 {
        return Err(DecodeError::NonCanonical);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base58btc_known_answers() {
        assert_eq!(base58btc_encode(b""), "");
        assert_eq!(base58btc_encode(b"foobar"), "t1Zv2yaZ");
        assert_eq!(base58btc_encode(&[0, 0, 1]), "112");
        assert_eq!(base58btc_encode(&[0]), "1");
        assert_eq!(base58btc_decode("112").unwrap(), vec![0, 0, 1]);
        assert_eq!(base58btc_decode("1").unwrap(), vec![0]);
        assert_eq!(base58btc_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn base58btc_rejects_out_of_alphabet() {
        for bad in ["0", "O", "I", "l", "z+", "é"] {
            let c = bad.chars().last().unwrap();
            assert_eq!(
                base58btc_decode(bad),
                Err(DecodeError::InvalidChar(c)),
                "{bad}"
            );
        }
    }

    #[test]
    fn base64url_known_answers() {
        // RFC 4648 §10 vectors, unpadded.
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg"),
            ("fooba", "Zm9vYmE"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64url_encode(plain.as_bytes()), enc);
            assert_eq!(base64url_decode(enc).unwrap(), plain.as_bytes());
        }
        assert_eq!(base64url_encode(&[0xFB, 0xFF]), "-_8");
    }

    #[test]
    fn base64url_rejects_padding_std_alphabet_and_second_forms() {
        assert_eq!(base64url_decode("Zg=="), Err(DecodeError::InvalidChar('=')));
        assert_eq!(base64url_decode("ab+c"), Err(DecodeError::InvalidChar('+')));
        assert_eq!(base64url_decode("ab/c"), Err(DecodeError::InvalidChar('/')));
        assert_eq!(base64url_decode("A"), Err(DecodeError::NonCanonical));
        assert_eq!(base64url_decode("AAAAA"), Err(DecodeError::NonCanonical));
        assert_eq!(base64url_decode("AB"), Err(DecodeError::NonCanonical));
        assert_eq!(base64url_decode("Zh"), Err(DecodeError::NonCanonical));
        assert_eq!(base64url_decode("AA").unwrap(), vec![0]);
    }

    #[test]
    fn round_trips() {
        let mut seed = 0x9E37_79B9u32;
        for len in 0..200usize {
            let v: Vec<u8> = (0..len)
                .map(|i| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    // Leading zeros exercise base58's '1' handling.
                    if i < len % 4 {
                        0
                    } else {
                        (seed >> 24) as u8
                    }
                })
                .collect();
            assert_eq!(base58btc_decode(&base58btc_encode(&v)).unwrap(), v);
            assert_eq!(base64url_decode(&base64url_encode(&v)).unwrap(), v);
        }
    }
}
