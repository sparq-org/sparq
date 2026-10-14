//! Entity-tag lists (RFC 9110 §8.8.3): the one parser every `If-Match` and `If-None-Match` reader
//! in this crate uses, on the field's bytes. An opaque tag may hold obs-text, so a field is never
//! read as a string first, and the only whitespace around list members is HTTP whitespace (space
//! and tab, §5.6.3). Each surface keeps its own policy for a list it cannot read.

use std::borrow::Cow;

use http::{HeaderMap, HeaderName};

/// An `If-Match` / `If-None-Match` field value: `*`, or entity tags as `(weak, opaque)`, the
/// opaque bytes without their quotes.
pub(crate) enum TagList<'a> {
    Any,
    Tags(Vec<(bool, &'a [u8])>),
}

impl<'a> TagList<'a> {
    /// Parse a field value by the RFC 9110 grammar (`"*" / #entity-tag`, section 8.8.3): a comma
    /// inside a quoted tag belongs to the tag, and `*` is the whole value or not a wildcard.
    /// Anything else is unreadable.
    pub(crate) fn parse(b: &'a [u8]) -> Result<Self, ()> {
        if is_wildcard(b) {
            return Ok(Self::Any);
        }
        let mut i = 0;
        let mut tags = Vec::new();
        loop {
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b',') {
                i += 1;
            }
            if i == b.len() {
                break;
            }
            let (tag, end) = tag_at(b, i)?;
            tags.push(tag);
            i = end;
            while i < b.len() && matches!(b[i], b' ' | b'\t') {
                i += 1;
            }
            if i < b.len() && b[i] != b',' {
                return Err(());
            }
        }
        // An empty list (`#entity-tag` allows one) names no tag: it matches nothing.
        Ok(Self::Tags(tags))
    }

    /// Whether the list matches `etag` (the server's own, quoted, possibly `W/`). `weak` compares
    /// opaque tags only (RFC 9110 section 8.8.3.2); otherwise both tags must be strong.
    pub(crate) fn matches(&self, etag: &str, weak: bool) -> bool {
        let (etag_weak, quoted) = match etag.strip_prefix("W/") {
            Some(o) => (true, o),
            None => (false, etag),
        };
        let Some(opaque) = quoted.strip_prefix('"').and_then(|o| o.strip_suffix('"')) else {
            return false;
        };
        match self {
            Self::Any => true,
            Self::Tags(tags) => tags
                .iter()
                .any(|&(w, t)| t == opaque.as_bytes() && (weak || (!w && !etag_weak))),
        }
    }
}

/// One entity tag (RFC 9110 §8.8.3), as `(weak, opaque)`, surrounded only by HTTP whitespace: an
/// `If-Range` value, which is never a list. A comma inside the quoted tag belongs to it; any other
/// comma, or anything else around the tag, is not one tag.
pub(crate) fn single(b: &[u8]) -> Option<(bool, &[u8])> {
    let start = b.iter().position(|c| !matches!(c, b' ' | b'\t'))?;
    let (tag, end) = tag_at(b, start).ok()?;
    b[end..]
        .iter()
        .all(|c| matches!(c, b' ' | b'\t'))
        .then_some(tag)
}

/// The entity tag starting at `b[i]`, and the index just past its closing quote.
fn tag_at(b: &[u8], mut i: usize) -> Result<((bool, &[u8]), usize), ()> {
    let weak = b[i..].starts_with(b"W/");
    if weak {
        i += 2;
    }
    if b.get(i) != Some(&b'"') {
        return Err(());
    }
    i += 1;
    let start = i;
    while i < b.len() && b[i] != b'"' {
        if !(b[i] == 0x21 || (0x23..=0x7e).contains(&b[i]) || b[i] >= 0x80) {
            return Err(());
        }
        i += 1;
    }
    if i == b.len() {
        return Err(());
    }
    Ok(((weak, &b[start..i]), i + 1))
}

/// Every field line of `name`, combined as one comma-separated list (RFC 9110 §5.3), or `None`
/// when the request carries none.
pub(crate) fn field(headers: &HeaderMap, name: HeaderName) -> Option<Cow<'_, [u8]>> {
    let mut lines = headers.get_all(name).into_iter();
    let first = lines.next()?.as_bytes();
    let Some(second) = lines.next() else {
        return Some(Cow::Borrowed(first));
    };
    let mut all = first.to_vec();
    for line in std::iter::once(second).chain(lines) {
        all.extend_from_slice(b", ");
        all.extend_from_slice(line.as_bytes());
    }
    Some(Cow::Owned(all))
}

/// Whether a field value is the wildcard `*`, surrounded only by HTTP whitespace.
pub(crate) fn is_wildcard(b: &[u8]) -> bool {
    let start = b.iter().position(|c| !matches!(c, b' ' | b'\t'));
    let end = b.iter().rposition(|c| !matches!(c, b' ' | b'\t'));
    match (start, end) {
        (Some(s), Some(e)) => &b[s..=e] == b"*",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(header: &str, etag: &str, weak: bool) -> bool {
        TagList::parse(header.as_bytes()).is_ok_and(|l| l.matches(etag, weak))
    }

    #[test]
    fn a_single_tag_is_one_tag_and_nothing_else() {
        assert_eq!(single(b"\"a\""), Some((false, &b"a"[..])));
        assert_eq!(single(b" \tW/\"a\" "), Some((true, &b"a"[..])));
        assert_eq!(single(b"\"a,b\""), Some((false, &b"a,b"[..])));
        assert_eq!(single(b"\"\x80\""), Some((false, &b"\x80"[..])));
        for not_one in [
            &b""[..],
            b" ",
            b"*",
            b"\"a\",",
            b",\"a\"",
            b"\"a\", \"b\"",
            b"\"a",
            b"a",
            b"\xc2\xa0\"a\"",
            b"\"a\"\xc2\xa0",
        ] {
            assert_eq!(single(not_one), None, "{not_one:?}");
        }
    }

    #[test]
    fn lists_wildcards_and_strength() {
        assert!(listed("\"a\", \"b\"", "\"b\"", false));
        assert!(listed("\"a\",\t\"b\"", "\"b\"", false));
        assert!(!listed("W/\"b\"", "\"b\"", false));
        assert!(listed("W/\"b\"", "\"b\"", true));
        assert!(!listed("\"b\"", "W/\"b\"", false));
        assert!(listed("*", "\"x\"", false));
        assert!(listed(" \t* ", "\"x\"", false));
        // A comma inside a tag is part of it, never a wildcard.
        assert!(!listed("\"x,*,y\"", "\"x\"", false));
        assert!(listed("\"x,*,y\"", "\"x,*,y\"", false));
        assert!(!listed("\"a\", *", "\"x\"", false));
        assert!(!listed("\"a", "\"a\"", false));
        assert!(!listed("a", "a", false));
        // An empty list matches nothing, and is not unreadable.
        assert!(!listed("", "\"x\"", true));
        assert!(TagList::parse(b" , ").is_ok());
    }

    #[test]
    fn obs_text_is_part_of_a_tag() {
        let mut obs = b"\"\xe9\", ".to_vec();
        obs.extend_from_slice(b"\"x\"");
        assert!(TagList::parse(&obs).is_ok_and(|l| l.matches("\"x\"", false)));
    }

    #[test]
    fn only_space_and_tab_surround_a_member() {
        // Unicode whitespace (here U+00A0 and U+3000) is not HTTP whitespace: the list is
        // unreadable, and the value is not a wildcard.
        for ws in ["\u{a0}", "\u{3000}", "\u{85}"] {
            let tag = format!("{ws}\"x\"{ws}");
            assert!(TagList::parse(tag.as_bytes()).is_err(), "{ws:?}");
            assert!(!is_wildcard(format!("{ws}*").as_bytes()), "{ws:?}");
            assert!(
                TagList::parse(format!("{ws}*").as_bytes()).is_err(),
                "{ws:?}"
            );
        }
    }

    #[test]
    fn field_lines_combine_into_one_list() {
        let mut h = HeaderMap::new();
        assert!(field(&h, http::header::IF_MATCH).is_none());
        h.append(http::header::IF_MATCH, "\"a\"".parse().unwrap());
        assert_eq!(
            field(&h, http::header::IF_MATCH).as_deref(),
            Some(&b"\"a\""[..])
        );
        h.append(
            http::header::IF_MATCH,
            http::HeaderValue::from_bytes(b"\"\xe9\"").unwrap(),
        );
        h.append(http::header::IF_MATCH, "\"b\"".parse().unwrap());
        let all = field(&h, http::header::IF_MATCH).unwrap();
        assert_eq!(&all[..], b"\"a\", \"\xe9\", \"b\"");
        assert!(TagList::parse(&all).is_ok_and(|l| l.matches("\"b\"", false)));
    }
}
