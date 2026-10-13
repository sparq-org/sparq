//! Base16 (hex) encoding shared by the digest, DID and multibase code paths, so each
//! crate does not keep its own per-byte `format!` loop.

const LOWER: &[u8; 16] = b"0123456789abcdef";
const UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// Appends `bytes` to `out` as hex, two digits per byte, in lowercase or `upper`case.
pub fn encode_into(bytes: &[u8], upper: bool, out: &mut String) {
    let table = if upper { UPPER } else { LOWER };
    out.reserve(bytes.len() * 2);
    for &b in bytes {
        out.push(table[usize::from(b >> 4)] as char);
        out.push(table[usize::from(b & 0x0f)] as char);
    }
}

/// `bytes` as lowercase hex.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    encode_into(bytes, false, &mut out);
    out
}

/// Decodes hex digits of either case; `None` on an odd length or a non-hex character.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    fn nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .chunks_exact(2)
        .map(|p| Some((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, encode_into};

    #[test]
    fn encodes_every_byte_and_round_trips() {
        let all: Vec<u8> = (0..=255).collect();
        let hex = encode(&all);
        assert_eq!(hex.len(), 512);
        for (i, b) in all.iter().enumerate() {
            assert_eq!(&hex[2 * i..2 * i + 2], format!("{b:02x}"));
        }
        assert_eq!(decode(&hex).unwrap(), all);
        let mut upper = String::from("x");
        encode_into(&all, true, &mut upper);
        assert_eq!(&upper[1..], hex.to_uppercase());
        assert_eq!(decode(&upper[1..]).unwrap(), all);
    }

    #[test]
    fn decode_rejects_odd_length_and_non_hex() {
        assert_eq!(decode(""), Some(vec![]));
        assert_eq!(decode("aB0f"), Some(vec![0xab, 0x0f]));
        assert_eq!(decode("abc"), None);
        assert_eq!(decode("0g"), None);
        assert_eq!(decode("0x00"), None);
        assert_eq!(decode("é0"), None);
    }
}
