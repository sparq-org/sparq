//! Percent-encoding helpers shared across the workspace (RFC 3986 §2.1).
//!
//! [`decode`] is the lenient decoder several crates had their own copy of: every valid
//! `%XX` pair becomes its byte, a stray `%` or malformed escape passes through verbatim,
//! and the decoded bytes are read as UTF-8 lossily. With `plus_as_space` it decodes an
//! `application/x-www-form-urlencoded` component (`+` → space). [`encode`] escapes every
//! byte outside the RFC 3986 unreserved set as uppercase `%XX`.

/// Percent-decodes `s`, leniently: valid `%XX` pairs (either hex case) decode to their
/// byte, a `%` not followed by two hex digits is kept verbatim, and the result is read as
/// UTF-8 with invalid sequences replaced by U+FFFD. With `plus_as_space`, `+` decodes to a
/// space (form-urlencoded); a `%2B` still decodes to `+`.
pub fn decode(s: &str, plus_as_space: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' if plus_as_space => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (nibble(bytes[i + 1]), nibble(bytes[i + 2])) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes `s` for a URL component: the RFC 3986 unreserved characters
/// (`A-Z a-z 0-9 - _ . ~`) pass through and every other byte becomes uppercase `%XX`.
pub fn encode(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[usize::from(b >> 4)] as char);
            out.push(HEX[usize::from(b & 0xF)] as char);
        }
    }
    out
}

fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_valid_escapes_either_case() {
        assert_eq!(decode("a%20b%2fc%2F", false), "a b/c/");
        assert_eq!(decode("caf%C3%A9", false), "café");
    }

    #[test]
    fn decode_keeps_malformed_and_truncated_escapes() {
        assert_eq!(decode("100%", false), "100%");
        assert_eq!(decode("%4", false), "%4");
        assert_eq!(decode("%zz%41", false), "%zzA");
        assert_eq!(decode("%%41", false), "%A");
    }

    #[test]
    fn decode_plus_only_in_form_mode() {
        assert_eq!(decode("a+b%2Bc", true), "a b+c");
        assert_eq!(decode("a+b%2Bc", false), "a+b+c");
    }

    #[test]
    fn decode_invalid_utf8_is_lossy() {
        assert_eq!(decode("%FF", false), "\u{FFFD}");
    }

    #[test]
    fn encode_escapes_all_but_unreserved() {
        assert_eq!(encode("aZ0-_.~"), "aZ0-_.~");
        assert_eq!(encode("{ }"), "%7B%20%7D");
        assert_eq!(encode("http://ex/g1"), "http%3A%2F%2Fex%2Fg1");
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn round_trip() {
        for s in ["", "plain", "a b+c/%", "SELECT * { ?s ?p \"é\" }", "%zz"] {
            assert_eq!(decode(&encode(s), false), s);
            assert_eq!(decode(&encode(s), true), s);
        }
    }
}
