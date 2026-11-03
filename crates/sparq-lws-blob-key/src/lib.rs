//! Shared opaque LWS blob-key grammar, with no dependencies and support for wasm.

/// Marks a logical service-pool key.
pub const POOL_MARK: &str = "$pool$";
/// Prefix of every bare blob key.
pub const PREFIX: &str = "b1-";

/// Accept exactly `($pool$)?b1-<32 lowercase hex>`, returning the bare key and pool flag.
pub fn parse(key: &str) -> Option<(&str, bool)> {
    let (bare, pooled) = key
        .strip_prefix(POOL_MARK)
        .map_or((key, false), |bare| (bare, true));
    let suffix = bare.strip_prefix(PREFIX)?;
    if suffix.len() != 32
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    Some((bare, pooled))
}

/// Format caller-supplied suffix bytes as a canonical logical key.
pub fn format(suffix: [u8; 16], pooled: bool) -> String {
    use std::fmt::Write as _;

    let mut key = String::with_capacity(POOL_MARK.len() + PREFIX.len() + 32);
    if pooled {
        key.push_str(POOL_MARK);
    }
    key.push_str(PREFIX);
    for byte in suffix {
        write!(key, "{byte:02x}").expect("writing to a String cannot fail");
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_preserves_suffix_bytes_and_pool_flag() {
        let suffix = [
            0x00, 0x01, 0x0f, 0x10, 0x7f, 0x80, 0xab, 0xff, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09,
        ];
        let bare = "b1-00010f107f80abff0203040506070809";
        assert_eq!(format(suffix, false), bare);
        assert_eq!(format(suffix, true), std::format!("$pool${bare}"));
        assert_eq!(parse(bare), Some((bare, false)));
        assert_eq!(parse(&format(suffix, true)), Some((bare, true)));
    }

    #[test]
    fn round_trip() {
        for byte in 0..=u8::MAX {
            for pooled in [false, true] {
                let key = format([byte; 16], pooled);
                let (bare, actual_pooled) = parse(&key).expect("formatted key must parse");
                assert_eq!(actual_pooled, pooled);
                assert_eq!(bare, format([byte; 16], false));
            }
        }
    }

    #[test]
    fn rejects_noncanonical_keys() {
        for key in [
            "",
            "0123456789abcdef0123456789abcdef",
            "b1-0123456789abcdef0123456789abcdeA",
            "b1-0123456789abcdef0123456789abcdeg",
            "b1-0123456789abcdef0123456789abcde",
            "b1-0123456789abcdef0123456789abcdef0",
            "$pool$$pool$b1-0123456789abcdef0123456789abcdef",
            "b1-0123456789abcdef0123456789abcdef/x",
            "b1-0123456789abcdef0123456789abcdef\n",
            "$pool$b1-0123456789abcdef0123456789abcdef/x",
            "$pool$0123456789abcdef0123456789abcdef",
            "b1-0123456789abcdef0123456789abcdé",
        ] {
            assert_eq!(parse(key), None, "accepted {key:?}");
        }
    }
}
