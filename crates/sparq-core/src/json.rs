//! JSON string escaping shared by every hand-written JSON serialiser in the workspace
//! (SPARQL results, SHACL and policy reports, explain plans, change streams). Callers that
//! carry no serde use this instead of keeping their own copy.

/// Appends `s` to `out` with JSON string escaping (no surrounding quotes): `"` and `\`,
/// the short escapes `\n` `\r` `\t`, and `\u00XX` (lowercase hex) for every other C0
/// control. Everything else, including DEL and non-ASCII, passes through verbatim.
///
/// Bulk-copies maximal runs of already-safe text and only handles the rare escape byte
/// individually, so the common no-escape value is a single `push_str`. Byte-level is safe:
/// every escaped byte is ASCII, so a UTF-8 multi-byte sequence is never split.
#[inline]
pub fn escape_into(out: &mut String, s: &str) {
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let esc: &str = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x00..=0x1f => {
                out.push_str(&s[start..i]);
                out.push_str("\\u00");
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        out.push_str(&s[start..i]);
        out.push_str(esc);
        start = i + 1;
    }
    out.push_str(&s[start..]);
}

/// Appends `s` to `out` as a quoted JSON string literal (see [`escape_into`]).
#[inline]
pub fn push_string(out: &mut String, s: &str) {
    out.reserve(s.len() + 2);
    out.push('"');
    escape_into(out, s);
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::{escape_into, push_string};

    fn esc(s: &str) -> String {
        let mut out = String::new();
        escape_into(&mut out, s);
        out
    }

    #[test]
    fn escapes_match_json_semantics() {
        assert_eq!(esc(""), "");
        assert_eq!(esc("plain text 123"), "plain text 123");
        assert_eq!(esc("a\"b"), "a\\\"b");
        assert_eq!(esc("a\\b"), "a\\\\b");
        assert_eq!(esc("a\nb\tc\rd"), "a\\nb\\tc\\rd");
        assert_eq!(esc("\u{0}\u{1f}\u{7}\u{8}\u{c}"), "\\u0000\\u001f\\u0007\\u0008\\u000c");
        assert_eq!(esc("café — 日本語 😀"), "café — 日本語 😀");
        assert_eq!(esc("http://ex/a\"x/é"), "http://ex/a\\\"x/é");
        assert_eq!(esc("\u{7f}"), "\u{7f}");
    }

    #[test]
    fn every_char_escapes_exactly_as_json_requires() {
        for c in (0u32..0x300).filter_map(char::from_u32) {
            let want = match c {
                '"' => "\\\"".to_string(),
                '\\' => "\\\\".to_string(),
                '\n' => "\\n".to_string(),
                '\r' => "\\r".to_string(),
                '\t' => "\\t".to_string(),
                c if (c as u32) < 0x20 => format!("\\u{:04x}", c as u32),
                c => c.to_string(),
            };
            let mut got = String::new();
            push_string(&mut got, &format!("a{c}b"));
            assert_eq!(got, format!("\"a{want}b\""), "U+{:04X}", c as u32);
        }
    }
}
