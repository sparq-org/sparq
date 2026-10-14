// AUTHORED-BY Claude Opus 4.8
//! `Range: bytes=…` request handling (RFC 9110 §14 and RFC 7233 §4.1).
//!
//! Pure value logic: given a `Range` header value and the resource length, compute the satisfied
//! byte interval(s), decide it is unsatisfiable (416), or decide the header should be ignored and
//! the full body returned (200). Multipart framing is also pure value logic.

// Multipart parsing and framing extension.
/// Boundary used by [`encode_multipart`].
pub const MULTIPART_BOUNDARY: &str = "sparq-lws-byte-boundary";

/// An inclusive byte interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// First byte offset.
    pub start: u64,
    /// Last byte offset.
    pub end: u64,
}

/// The outcome of evaluating a `Range` header against a resource of known length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeOutcome {
    /// No (usable) Range — serve the full body with 200.
    Full,
    /// A single satisfiable range `[start, end]` inclusive — serve 206 + `Content-Range`.
    Satisfied { start: u64, end: u64 },
    /// Multiple satisfiable, non-overlapping ranges, ordered by their start offset.
    Multipart(Vec<ByteRange>),
    /// The range was syntactically valid but cannot be satisfied for this length — 416.
    Unsatisfiable,
}

impl RangeOutcome {
    /// The inclusive `[start, end]` of a satisfied range, if any.
    pub fn interval(self) -> Option<(u64, u64)> {
        match self {
            RangeOutcome::Satisfied { start, end } => Some((start, end)),
            _ => None,
        }
    }
}

/// Encode a `multipart/byteranges` body using a stable boundary.
#[must_use]
pub fn encode_multipart(body: &[u8], content_type: &str, ranges: &[ByteRange]) -> Vec<u8> {
    let mut out = Vec::new();
    for range in ranges {
        out.extend_from_slice(format!("--{MULTIPART_BOUNDARY}\r\n").as_bytes());
        out.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        out.extend_from_slice(
            format!(
                "Content-Range: bytes {}-{}/{}\r\n\r\n",
                range.start,
                range.end,
                body.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&body[range.start as usize..=range.end as usize]);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{MULTIPART_BOUNDARY}--\r\n").as_bytes());
    out
}

/// Evaluate a `Range` header value against a resource of `len` bytes.
///
/// Only `bytes` ranges are understood; any other unit is ignored (→ [`RangeOutcome::Full`]). See the
/// Invalid single ranges retain the historical behavior of being ignored. Invalid multi-range
/// requests fail closed as unsatisfiable.
pub fn evaluate(range: Option<&str>, len: u64) -> RangeOutcome {
    let raw = match range {
        None => return RangeOutcome::Full,
        Some(r) => r.trim(),
    };

    // Must be `bytes=<ranges>`; any other unit ⇒ ignore.
    let specs = match raw.strip_prefix("bytes=") {
        Some(s) => s.trim(),
        None => return RangeOutcome::Full,
    };

    if specs.contains(',') {
        if len == 0 {
            return RangeOutcome::Unsatisfiable;
        }
        let mut ranges = Vec::new();
        for spec in specs.split(',') {
            match parse_interval(spec.trim(), len) {
                Ok(Some(range)) => ranges.push(range),
                Ok(None) => {}
                Err(()) => return RangeOutcome::Unsatisfiable,
            }
        }
        if ranges.is_empty() {
            return RangeOutcome::Unsatisfiable;
        }
        ranges.sort_unstable_by_key(|range| range.start);
        if ranges.windows(2).any(|pair| pair[1].start <= pair[0].end) {
            return RangeOutcome::Unsatisfiable;
        }
        if ranges.len() == 1 {
            let range = ranges[0];
            return RangeOutcome::Satisfied {
                start: range.start,
                end: range.end,
            };
        }
        return RangeOutcome::Multipart(ranges);
    }

    let spec = specs.trim();
    let (first, last) = match spec.split_once('-') {
        Some(parts) => parts,
        None => return RangeOutcome::Full, // not a valid range-spec ⇒ ignore.
    };
    let first = first.trim();
    let last = last.trim();

    // A zero-length resource cannot satisfy any byte range.
    if len == 0 {
        // A suffix or any range against an empty resource is unsatisfiable per RFC 9110 §14.1.2.
        return RangeOutcome::Unsatisfiable;
    }

    let last_index = len - 1;

    match (first.is_empty(), last.is_empty()) {
        // `bytes=-N` — the final N bytes (suffix range).
        (true, false) => {
            let n: u64 = match last.parse() {
                Ok(n) => n,
                Err(_) => return RangeOutcome::Full, // malformed ⇒ ignore.
            };
            if n == 0 {
                // `bytes=-0` is unsatisfiable (RFC 9110 §14.1.2: a suffix-length of 0 is invalid).
                return RangeOutcome::Unsatisfiable;
            }
            let start = len.saturating_sub(n);
            RangeOutcome::Satisfied {
                start,
                end: last_index,
            }
        }
        // `bytes=A-` — from A to the end.
        (false, true) => {
            let start: u64 = match first.parse() {
                Ok(s) => s,
                Err(_) => return RangeOutcome::Full,
            };
            if start > last_index {
                return RangeOutcome::Unsatisfiable;
            }
            RangeOutcome::Satisfied {
                start,
                end: last_index,
            }
        }
        // `bytes=A-B` — A through B inclusive.
        (false, false) => {
            let start: u64 = match first.parse() {
                Ok(s) => s,
                Err(_) => return RangeOutcome::Full,
            };
            let end_req: u64 = match last.parse() {
                Ok(e) => e,
                Err(_) => return RangeOutcome::Full,
            };
            if start > end_req {
                // An inverted range is invalid ⇒ ignore the header.
                return RangeOutcome::Full;
            }
            if start > last_index {
                return RangeOutcome::Unsatisfiable;
            }
            // Clamp the end to the last byte (RFC 9110 §14.1.2: a too-large end is clamped, not 416).
            let end = end_req.min(last_index);
            RangeOutcome::Satisfied { start, end }
        }
        // `bytes=-` — both empty: malformed ⇒ ignore.
        (true, true) => RangeOutcome::Full,
    }
}

fn parse_interval(spec: &str, len: u64) -> Result<Option<ByteRange>, ()> {
    let (first, last) = spec.split_once('-').ok_or(())?;
    let first = first.trim();
    let last = last.trim();
    if len == 0 || (first.is_empty() && last.is_empty()) {
        return Err(());
    }
    let last_index = len - 1;
    let range = match (first.is_empty(), last.is_empty()) {
        (true, false) => {
            let suffix: u64 = last.parse().map_err(|_| ())?;
            if suffix == 0 {
                return Ok(None);
            }
            ByteRange {
                start: len.saturating_sub(suffix),
                end: last_index,
            }
        }
        (false, true) => {
            let start: u64 = first.parse().map_err(|_| ())?;
            if start > last_index {
                return Ok(None);
            }
            ByteRange {
                start,
                end: last_index,
            }
        }
        (false, false) => {
            let start: u64 = first.parse().map_err(|_| ())?;
            let requested_end: u64 = last.parse().map_err(|_| ())?;
            if start > requested_end {
                return Err(());
            }
            if start > last_index {
                return Ok(None);
            }
            ByteRange {
                start,
                end: requested_end.min(last_index),
            }
        }
        (true, true) => unreachable!(),
    };
    Ok(Some(range))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_range_is_full() {
        assert_eq!(evaluate(None, 10), RangeOutcome::Full);
    }

    #[test]
    fn closed_range() {
        assert_eq!(
            evaluate(Some("bytes=2-5"), 10),
            RangeOutcome::Satisfied { start: 2, end: 5 }
        );
    }

    #[test]
    fn open_ended_range_to_end() {
        assert_eq!(
            evaluate(Some("bytes=3-"), 10),
            RangeOutcome::Satisfied { start: 3, end: 9 }
        );
    }

    #[test]
    fn suffix_range() {
        assert_eq!(
            evaluate(Some("bytes=-4"), 10),
            RangeOutcome::Satisfied { start: 6, end: 9 }
        );
        // A suffix larger than the resource clamps to the whole resource.
        assert_eq!(
            evaluate(Some("bytes=-100"), 10),
            RangeOutcome::Satisfied { start: 0, end: 9 }
        );
    }

    #[test]
    fn end_is_clamped_to_last_byte() {
        assert_eq!(
            evaluate(Some("bytes=5-1000"), 10),
            RangeOutcome::Satisfied { start: 5, end: 9 }
        );
    }

    #[test]
    fn start_past_end_is_unsatisfiable() {
        assert_eq!(
            evaluate(Some("bytes=20-30"), 10),
            RangeOutcome::Unsatisfiable
        );
        assert_eq!(evaluate(Some("bytes=10-"), 10), RangeOutcome::Unsatisfiable);
    }

    #[test]
    fn zero_length_resource_is_unsatisfiable() {
        assert_eq!(evaluate(Some("bytes=0-0"), 0), RangeOutcome::Unsatisfiable);
    }

    #[test]
    fn suffix_zero_is_unsatisfiable() {
        assert_eq!(evaluate(Some("bytes=-0"), 10), RangeOutcome::Unsatisfiable);
    }

    #[test]
    fn inverted_range_is_ignored() {
        assert_eq!(evaluate(Some("bytes=8-2"), 10), RangeOutcome::Full);
    }

    #[test]
    fn non_bytes_unit_is_ignored() {
        assert_eq!(evaluate(Some("items=0-5"), 10), RangeOutcome::Full);
    }

    #[test]
    fn multiple_ranges_are_returned_in_order() {
        assert_eq!(
            evaluate(Some("bytes=3-4,0-1"), 10),
            RangeOutcome::Multipart(vec![
                ByteRange { start: 0, end: 1 },
                ByteRange { start: 3, end: 4 },
            ])
        );
    }

    #[test]
    fn malformed_is_ignored() {
        assert_eq!(evaluate(Some("bytes=abc-def"), 10), RangeOutcome::Full);
        assert_eq!(evaluate(Some("bytes=-"), 10), RangeOutcome::Full);
        assert_eq!(evaluate(Some("garbage"), 10), RangeOutcome::Full);
    }

    #[test]
    fn interval_accessor() {
        assert_eq!(
            RangeOutcome::Satisfied { start: 1, end: 3 }.interval(),
            Some((1, 3))
        );
        assert_eq!(RangeOutcome::Full.interval(), None);
        assert_eq!(RangeOutcome::Unsatisfiable.interval(), None);
    }
}
