//! Canonical-encoding building blocks shared by every wire document in this
//! crate: strict unpadded base64url, shortest-form decimal counters, RFC 8785
//! (JCS) string escaping, and strict [`serde_json::Value`] walkers.
//!
//! The canonical byte form of every document is defined **only** by
//! the writers in this crate (object keys sorted, no whitespace, all scalars
//! strings). `serde_json` is used purely as a strict *reader*; decoding then
//! re-encodes through these writers and byte-compares against the input, so a
//! second byte representation of the same identity is always rejected
//! (`CRDT-WIRE-3`, `CRDT-UPD-RETRY-1`) without this crate having to trust any
//! third-party serialiser's output details.

use crate::CrdtError;
use crate::id::{Dot, ReplicaId};
use crate::summary::CausalSummary;
use serde_json::Value;
use std::collections::BTreeMap;

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encodes bytes as unpadded base64url (RFC 4648 §5).
pub fn b64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let word = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64URL[(word >> 18) as usize & 63] as char);
        out.push(B64URL[(word >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(B64URL[(word >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(B64URL[word as usize & 63] as char);
        }
    }
    out
}

fn b64url_value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a' + 26) as u32),
        b'0'..=b'9' => Some((c - b'0' + 52) as u32),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

/// Decodes strict unpadded base64url: rejects padding, characters outside the
/// alphabet, impossible lengths (≡ 1 mod 4), and non-zero trailing bits, so
/// exactly one text form per byte string is accepted.
pub fn b64url_decode(s: &str) -> Result<Vec<u8>, CrdtError> {
    let non_canonical = |reason: &str| CrdtError::NonCanonical {
        reason: format!("base64url {reason}"),
    };
    let bytes = s.as_bytes();
    if bytes.len() % 4 == 1 {
        return Err(non_canonical("length is impossible for any byte string"));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    for group in bytes.chunks(4) {
        let mut word: u32 = 0;
        for &c in group {
            let v = b64url_value(c)
                .ok_or_else(|| non_canonical("contains a character outside the alphabet"))?;
            word = (word << 6) | v;
        }
        match group.len() {
            4 => {
                out.push((word >> 16) as u8);
                out.push((word >> 8) as u8);
                out.push(word as u8);
            }
            3 => {
                // 18 significant bits carry 2 bytes; low 2 bits must be zero.
                if word & 0b11 != 0 {
                    return Err(non_canonical("has non-zero trailing bits"));
                }
                out.push((word >> 10) as u8);
                out.push((word >> 2) as u8);
            }
            2 => {
                // 12 significant bits carry 1 byte; low 4 bits must be zero.
                if word & 0b1111 != 0 {
                    return Err(non_canonical("has non-zero trailing bits"));
                }
                out.push((word >> 4) as u8);
            }
            _ => unreachable!("length % 4 == 1 rejected above"),
        }
    }
    Ok(out)
}

/// Parses a shortest-form decimal `u64`: non-empty, ASCII digits only, no
/// leading zero (except `"0"` itself), no sign, must fit in `u64`.
pub fn parse_dec_u64(s: &str) -> Result<u64, CrdtError> {
    let bad = |reason: &str| CrdtError::NonCanonical {
        reason: format!("decimal counter {s:?} {reason}"),
    };
    if s.is_empty() {
        return Err(bad("is empty"));
    }
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("contains a non-digit"));
    }
    if s.len() > 1 && s.starts_with('0') {
        return Err(bad("has a leading zero"));
    }
    s.parse::<u64>().map_err(|_| bad("does not fit in u64"))
}

/// Appends the JCS (RFC 8785) encoding of `s` as a JSON string: the two-char
/// escapes for `"` `\` and the BTNFR controls, `\u00xx` (lowercase hex) for
/// the remaining C0 controls, every other character literal UTF-8.
pub fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{000C}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                write!(out, "\\u{:04x}", c as u32).expect("writing to a String cannot fail");
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Appends a dot as the canonical two-element array `["<b64url>","<dec>"]`.
pub(crate) fn write_dot(out: &mut String, dot: &Dot) {
    out.push('[');
    write_json_string(out, &dot.replica().to_base64url());
    out.push(',');
    write_json_string(out, &dot.counter().to_string());
    out.push(']');
}

/// Appends a causal summary as the canonical
/// `{"clock":{"<b64url>":"<dec>",…},"cloud":[["<b64url>","<dec>"],…]}`
/// object. Clock keys are emitted in encoded-string order (the JCS key order;
/// base64url is ASCII so UTF-16 and UTF-8 orders coincide); cloud entries are
/// emitted in raw-replica-byte order as `CRDT-WIRE-3` requires.
pub(crate) fn write_summary(out: &mut String, summary: &CausalSummary) {
    out.push_str("{\"clock\":{");
    let mut entries: Vec<(String, String)> = summary
        .clock()
        .iter()
        .map(|(r, n)| (r.to_base64url(), n.to_string()))
        .collect();
    entries.sort();
    for (i, (key, value)) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_string(out, key);
        out.push(':');
        write_json_string(out, value);
    }
    out.push_str("},\"cloud\":[");
    for (i, dot) in summary.cloud().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_dot(out, dot);
    }
    out.push_str("]}");
}

// ---------------------------------------------------------------------------
// Strict serde_json::Value walkers (reader side).
// ---------------------------------------------------------------------------

/// Requires `v` to be an object with exactly `keys` (sorted or not — key
/// *order* is enforced by the byte-comparison, presence/absence here) and
/// returns it.
pub(crate) fn expect_object<'v>(
    v: &'v Value,
    what: &'static str,
    keys: &[&str],
) -> Result<&'v serde_json::Map<String, Value>, CrdtError> {
    let map = v.as_object().ok_or(CrdtError::Invalid {
        what,
        reason: "expected a JSON object".into(),
    })?;
    if map.len() != keys.len() || !keys.iter().all(|k| map.contains_key(*k)) {
        let found: Vec<&str> = map.keys().map(String::as_str).collect();
        return Err(CrdtError::Invalid {
            what,
            reason: format!("expected exactly the keys {keys:?}, found {found:?}"),
        });
    }
    Ok(map)
}

/// Requires `map[key]` to be a JSON string and returns it.
pub(crate) fn expect_str<'v>(
    map: &'v serde_json::Map<String, Value>,
    what: &'static str,
    key: &str,
) -> Result<&'v str, CrdtError> {
    map.get(key).and_then(Value::as_str).ok_or(CrdtError::Invalid {
        what,
        reason: format!("field {key:?} must be a JSON string"),
    })
}

/// Requires `map[key]` to be a JSON array and returns it.
pub(crate) fn expect_array<'v>(
    map: &'v serde_json::Map<String, Value>,
    what: &'static str,
    key: &str,
) -> Result<&'v Vec<Value>, CrdtError> {
    map.get(key).and_then(Value::as_array).ok_or(CrdtError::Invalid {
        what,
        reason: format!("field {key:?} must be a JSON array"),
    })
}

/// Parses a `["<b64url replica>","<dec counter>"]` value into a [`Dot`].
pub(crate) fn parse_dot(v: &Value, what: &'static str) -> Result<Dot, CrdtError> {
    let pair = v.as_array().ok_or(CrdtError::Invalid {
        what,
        reason: "a dot must be a two-element JSON array".into(),
    })?;
    if pair.len() != 2 {
        return Err(CrdtError::Invalid {
            what,
            reason: format!("a dot must have exactly 2 elements, found {}", pair.len()),
        });
    }
    let replica = pair[0].as_str().ok_or(CrdtError::Invalid {
        what,
        reason: "dot replica must be a JSON string".into(),
    })?;
    let counter = pair[1].as_str().ok_or(CrdtError::Invalid {
        what,
        reason: "dot counter must be a JSON string".into(),
    })?;
    Dot::new(ReplicaId::from_base64url(replica)?, parse_dec_u64(counter)?)
}

/// Parses a `{"clock":…,"cloud":…}` value into a normalised
/// [`CausalSummary`], rejecting unnormalised or oversized input.
pub(crate) fn parse_summary(
    v: &Value,
    what: &'static str,
    max_clock_entries: usize,
    max_cloud_dots: usize,
) -> Result<CausalSummary, CrdtError> {
    let map = expect_object(v, what, &["clock", "cloud"])?;
    let clock_map = map.get("clock").and_then(Value::as_object).ok_or(CrdtError::Invalid {
        what,
        reason: "field \"clock\" must be a JSON object".into(),
    })?;
    if clock_map.len() > max_clock_entries {
        return Err(CrdtError::Oversized {
            what: "summary clock entries",
            len: clock_map.len(),
            max: max_clock_entries,
        });
    }
    let mut clock = BTreeMap::new();
    for (key, value) in clock_map {
        let replica = ReplicaId::from_base64url(key)?;
        let text = value.as_str().ok_or(CrdtError::Invalid {
            what,
            reason: "clock values must be JSON strings".into(),
        })?;
        let n = parse_dec_u64(text)?;
        if n == 0 {
            return Err(CrdtError::NonCanonical {
                reason: "a clock entry of 0 must be omitted".into(),
            });
        }
        clock.insert(replica, n);
    }
    let cloud_arr = expect_array(map, what, "cloud")?;
    if cloud_arr.len() > max_cloud_dots {
        return Err(CrdtError::Oversized {
            what: "summary cloud dots",
            len: cloud_arr.len(),
            max: max_cloud_dots,
        });
    }
    let mut cloud = Vec::with_capacity(cloud_arr.len());
    for item in cloud_arr {
        cloud.push(parse_dot(item, what)?);
    }
    require_strictly_ascending(&cloud, "summary cloud")?;
    CausalSummary::from_parts(clock, cloud)
}

/// Rejects a slice that is not strictly ascending (which also rejects
/// duplicates), per the canonical array-sorting rules of `CRDT-WIRE-3`.
pub(crate) fn require_strictly_ascending<T: Ord>(
    items: &[T],
    what: &str,
) -> Result<(), CrdtError> {
    for window in items.windows(2) {
        if window[0] >= window[1] {
            return Err(CrdtError::NonCanonical {
                reason: format!("{what} array is not strictly sorted / duplicate-free"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(bytes: &[u8]) -> ReplicaId {
        ReplicaId::new(bytes.to_vec()).expect("valid replica id")
    }

    #[test]
    fn b64url_encode_matches_rfc4648_vectors() {
        assert_eq!(b64url_encode(b""), "");
        assert_eq!(b64url_encode(b"f"), "Zg");
        assert_eq!(b64url_encode(b"fo"), "Zm8");
        assert_eq!(b64url_encode(b"foo"), "Zm9v");
        assert_eq!(b64url_encode(b"foob"), "Zm9vYg");
        assert_eq!(b64url_encode(&[0xFB, 0xFF]), "-_8");
    }

    #[test]
    fn b64url_decode_round_trips_and_rejects_second_forms() {
        for input in [b"".as_slice(), b"f", b"fo", b"foo", b"foob", &[0xFB, 0xFF]] {
            assert_eq!(b64url_decode(&b64url_encode(input)).unwrap(), input);
        }
        assert!(b64url_decode("Zg=").is_err()); // padding is outside the alphabet
        assert!(b64url_decode("Zh").is_err()); // trailing bits set: "Zg" is canonical
        assert!(b64url_decode("Zm9").is_err()); // trailing bits set: "Zm8" is canonical
        assert!(b64url_decode("A").is_err()); // impossible length
        assert!(b64url_decode("Z+").is_err()); // '+' is base64, not base64url
    }

    #[test]
    fn parse_dec_u64_accepts_only_shortest_form() {
        assert_eq!(parse_dec_u64("0").unwrap(), 0);
        assert_eq!(parse_dec_u64("42").unwrap(), 42);
        assert_eq!(parse_dec_u64("18446744073709551615").unwrap(), u64::MAX);
        for bad in ["", "01", "+1", "-1", "1.0", "1e3", " 1", "18446744073709551616"] {
            assert!(parse_dec_u64(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn write_json_string_uses_jcs_escapes() {
        let mut out = String::new();
        write_json_string(&mut out, "a\"b\\c\u{8}\t\n\u{c}\r\u{1f}é€");
        assert_eq!(out, "\"a\\\"b\\\\c\\b\\t\\n\\f\\r\\u001fé€\"");
    }

    #[test]
    fn write_dot_emits_canonical_pair() {
        let mut out = String::new();
        write_dot(&mut out, &Dot::new(rid(b"peer-a"), 88).unwrap());
        assert_eq!(out, "[\"cGVlci1h\",\"88\"]");
    }

    #[test]
    fn write_summary_emits_clock_then_cloud_sorted() {
        let mut s = CausalSummary::new();
        s.insert(Dot::new(rid(b"peer-a"), 1).unwrap());
        s.insert(Dot::new(rid(b"peer-b"), 2).unwrap()); // gap ⇒ cloud
        let mut out = String::new();
        write_summary(&mut out, &s);
        assert_eq!(
            out,
            "{\"clock\":{\"cGVlci1h\":\"1\"},\"cloud\":[[\"cGVlci1i\",\"2\"]]}"
        );
    }

    #[test]
    fn parse_summary_round_trips_and_rejects_unnormalised() {
        let text = "{\"clock\":{\"cGVlci1h\":\"3\"},\"cloud\":[[\"cGVlci1h\",\"5\"]]}";
        let v: Value = serde_json::from_str(text).unwrap();
        let summary = parse_summary(&v, "test", 16, 16).unwrap();
        let mut out = String::new();
        write_summary(&mut out, &summary);
        assert_eq!(out, text);

        // (peer-a, 4) is clock+1 and must have been absorbed into the clock.
        let bad = "{\"clock\":{\"cGVlci1h\":\"3\"},\"cloud\":[[\"cGVlci1h\",\"4\"]]}";
        let v: Value = serde_json::from_str(bad).unwrap();
        assert!(parse_summary(&v, "test", 16, 16).is_err());

        // Zero clock entries must be omitted, not encoded.
        let zero = "{\"clock\":{\"cGVlci1h\":\"0\"},\"cloud\":[]}";
        let v: Value = serde_json::from_str(zero).unwrap();
        assert!(parse_summary(&v, "test", 16, 16).is_err());

        // Bounds are enforced before allocation of the summary.
        let v: Value = serde_json::from_str(text).unwrap();
        assert!(matches!(
            parse_summary(&v, "test", 0, 16),
            Err(CrdtError::Oversized { .. })
        ));
        assert!(matches!(
            parse_summary(&v, "test", 16, 0),
            Err(CrdtError::Oversized { .. })
        ));
    }

    #[test]
    fn parse_dot_requires_two_string_elements() {
        let good: Value = serde_json::from_str("[\"cGVlci1h\",\"1\"]").unwrap();
        assert_eq!(parse_dot(&good, "test").unwrap(), Dot::new(rid(b"peer-a"), 1).unwrap());
        for bad in ["[\"cGVlci1h\"]", "[\"cGVlci1h\",1]", "[\"cGVlci1h\",\"0\"]", "\"x\""] {
            let v: Value = serde_json::from_str(bad).unwrap();
            assert!(parse_dot(&v, "test").is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn expect_object_str_array_reject_shape_mismatches() {
        let v: Value = serde_json::from_str("{\"a\":\"x\",\"b\":[]}").unwrap();
        let map = expect_object(&v, "test", &["a", "b"]).unwrap();
        assert_eq!(expect_str(map, "test", "a").unwrap(), "x");
        assert!(expect_array(map, "test", "b").unwrap().is_empty());
        assert!(expect_object(&v, "test", &["a"]).is_err());
        assert!(expect_str(map, "test", "b").is_err());
        assert!(expect_array(map, "test", "a").is_err());
        let arr: Value = serde_json::from_str("[]").unwrap();
        assert!(expect_object(&arr, "test", &[]).is_err());
    }

    #[test]
    fn require_strictly_ascending_rejects_disorder_and_duplicates() {
        assert!(require_strictly_ascending(&[1, 2, 3], "t").is_ok());
        assert!(require_strictly_ascending(&[1, 1], "t").is_err());
        assert!(require_strictly_ascending(&[2, 1], "t").is_err());
        assert!(require_strictly_ascending::<u32>(&[], "t").is_ok());
    }
}
