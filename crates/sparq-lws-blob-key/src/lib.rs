//! Shared opaque LWS blob-key grammar, with no dependencies and support for wasm.

/// Marks a logical service-pool key.
pub const POOL_MARK: &str = "$pool$";
/// Prefix of a plaintext blob key.
pub const PREFIX: &str = "b1-";
/// Prefix of a blob key requiring an encrypted envelope.
pub const ENVELOPE_PREFIX: &str = "e1-";

/// Accept exactly `($pool$)?(b1|e1)-<32 lowercase hex>`, returning the bare key and pool flag.
pub fn parse(key: &str) -> Option<(&str, bool)> {
    let (bare, pooled) = key
        .strip_prefix(POOL_MARK)
        .map_or((key, false), |bare| (bare, true));
    let suffix = bare
        .strip_prefix(PREFIX)
        .or_else(|| bare.strip_prefix(ENVELOPE_PREFIX))?;
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
/// `envelope` selects `e1-`; otherwise the key uses `b1-`.
pub fn format(suffix: [u8; 16], pooled: bool, envelope: bool) -> String {
    use std::fmt::Write as _;

    let mut key = String::with_capacity(POOL_MARK.len() + PREFIX.len() + 32);
    if pooled {
        key.push_str(POOL_MARK);
    }
    key.push_str(if envelope { ENVELOPE_PREFIX } else { PREFIX });
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
        assert_eq!(format(suffix, false, false), bare);
        assert_eq!(format(suffix, true, false), std::format!("$pool${bare}"));
        assert_eq!(parse(bare), Some((bare, false)));
        assert_eq!(parse(&format(suffix, true, false)), Some((bare, true)));
    }

    #[test]
    fn envelope_format_preserves_suffix_bytes_and_pool_flag() {
        let bare = "e1-abababababababababababababababab";
        assert_eq!(format([0xab; 16], false, true), bare);
        assert_eq!(format([0xab; 16], true, true), std::format!("$pool${bare}"));
        assert_eq!(parse(bare), Some((bare, false)));
        assert_eq!(parse(&format([0xab; 16], true, true)), Some((bare, true)));
    }

    #[test]
    fn rejects_malformed_envelope_keys() {
        for bare in [
            "e1-",
            "e1-0123456789abcdef0123456789abcde",
            "e1-0123456789abcdef0123456789abcdef0",
            "e1-0123456789abcdef0123456789abcdeA",
            "e1-0123456789abcdef0123456789abcdeg",
            "e1-0123456789abcdef0123456789abcdé",
            "e1-0123456789abcdef0123456789abcdef/../x",
            "e1-0123456789abcdef0123456789abcdef\n",
            "$pool$e1-0123456789abcdef0123456789abcdef",
            "e2-0123456789abcdef0123456789abcdef",
        ] {
            assert_eq!(
                parse(&std::format!("$pool${bare}")),
                None,
                "accepted pooled {bare:?}"
            );
            if !bare.starts_with(POOL_MARK) {
                assert_eq!(parse(bare), None, "accepted {bare:?}");
            }
        }
    }

    #[test]
    fn round_trip() {
        for byte in 0..=u8::MAX {
            for (pooled, envelope) in [(false, false), (true, false), (false, true), (true, true)] {
                let key = format([byte; 16], pooled, envelope);
                let (bare, actual_pooled) = parse(&key).expect("formatted key must parse");
                assert_eq!(actual_pooled, pooled);
                assert_eq!(bare, format([byte; 16], false, envelope));
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
