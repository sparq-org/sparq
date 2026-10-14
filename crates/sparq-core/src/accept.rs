//! The `Accept` header grammar every sparq HTTP surface shares (RFC 9110 §12.5.1): one parser
//! for media ranges, their weights and their parameters, so content negotiation in the SPARQL
//! server, the Solid / LWS server and the SPARQL endpoints cannot drift apart. Each surface keeps
//! its own table of producible formats and its own policy (default format, `406` or fallback,
//! tie-breaks); only the grammar lives here. Standard library only, so it also builds for wasm.

/// One media range of an `Accept` header.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaRange<'a> {
    /// The lowercased `type/subtype` (`*/*`, `text/*` or a concrete type).
    pub essence: String,
    /// The weight in `[0, 1]`: `1` when absent, `0` when not a valid qvalue (RFC 9110 §12.4.2),
    /// so a malformed weight never makes a range acceptable.
    pub q: f32,
    params: Vec<(String, std::borrow::Cow<'a, str>)>,
}

impl<'a> MediaRange<'a> {
    /// The value of the parameter `name` (matched case-insensitively); a quoted-string is
    /// unquoted and its quoted-pairs decoded (RFC 9110 §5.6.4). Parameters after the weight are
    /// accept-extensions and are kept too.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_ref())
    }

    /// How specifically this range names `media_type` (a lowercase `type/subtype`): `3` for the
    /// type itself, `2` for `type/*`, `1` for `*/*`, `None` when the range does not cover it.
    pub fn specificity(&self, media_type: &str) -> Option<u8> {
        if self.essence == media_type {
            return Some(3);
        }
        if self.essence == "*/*" {
            return Some(1);
        }
        let (kind, _) = media_type.split_once('/')?;
        let (range_kind, sub) = self.essence.split_once('/')?;
        (sub == "*" && range_kind == kind).then_some(2)
    }
}

/// The media ranges of an `Accept` header value, in order. Empty members (`a, , b`) are skipped,
/// commas and semicolons inside a quoted parameter value do not split it, and whitespace around
/// `;` and `=` is allowed.
pub fn media_ranges(accept: &str) -> impl Iterator<Item = MediaRange<'_>> {
    split_unquoted(accept, b',').filter_map(|member| {
        let mut parts = split_unquoted(member, b';');
        let essence = parts.next()?.trim().to_ascii_lowercase();
        if essence.is_empty() {
            return None;
        }
        let mut q = 1.0;
        let mut params = Vec::new();
        for part in parts {
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let value = unquote(value.trim());
            if name == "q" {
                q = qvalue(&value).unwrap_or(0.0);
            } else {
                params.push((name, value));
            }
        }
        Some(MediaRange { essence, q, params })
    })
}

/// A weight (RFC 9110 §12.4.2): `0` or `1`, optionally followed by up to three decimals, never
/// above `1`.
fn qvalue(v: &str) -> Option<f32> {
    let (int, frac) = v.split_once('.').unwrap_or((v, ""));
    let valid = matches!(int, "0" | "1")
        && frac.len() <= 3
        && frac.bytes().all(|b| b.is_ascii_digit())
        && (int == "0" || frac.bytes().all(|b| b == b'0'));
    valid.then(|| v.parse().ok()).flatten()
}

/// `value` as a token, or the content of a quoted-string with each quoted-pair (`\x`) read as
/// the character it escapes (RFC 9110 §5.6.4).
fn unquote(value: &str) -> std::borrow::Cow<'_, str> {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.into();
    };
    if !inner.contains('\\') {
        return inner.into();
    }
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            chars.next().unwrap_or(c)
        } else {
            c
        });
    }
    out.into()
}

/// `s` split at each `sep` outside a quoted-string.
fn split_unquoted(s: &str, sep: u8) -> impl Iterator<Item = &str> {
    let mut rest = Some(s);
    std::iter::from_fn(move || {
        let s = rest?;
        let (mut quoted, mut escaped) = (false, false);
        for (i, b) in s.bytes().enumerate() {
            match b {
                _ if escaped => escaped = false,
                b'\\' if quoted => escaped = true,
                b'"' => quoted = !quoted,
                _ if b == sep && !quoted => {
                    rest = Some(&s[i + 1..]);
                    return Some(&s[..i]);
                }
                _ => {}
            }
        }
        rest = None;
        Some(s)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(accept: &str) -> Vec<(String, f32)> {
        media_ranges(accept).map(|r| (r.essence, r.q)).collect()
    }

    #[test]
    fn ranges_weights_and_parameters() {
        assert_eq!(
            ranges("Text/Turtle;q=0.5, application/ld+json , */*;Q=0"),
            [
                ("text/turtle".into(), 0.5),
                ("application/ld+json".into(), 1.0),
                ("*/*".into(), 0.0)
            ]
        );
        let r = media_ranges(r#"application/ld+json ; profile = "a b" ;q=0.9"#)
            .next()
            .unwrap();
        assert_eq!((r.param("PROFILE"), r.q), (Some("a b"), 0.9));
        assert_eq!(ranges(" , text/csv,,"), [("text/csv".into(), 1.0)]);
    }

    #[test]
    fn an_invalid_weight_is_zero() {
        for bad in ["x", "1.5", "2", "-1", "0.1234", "1.001", ".5", ""] {
            assert_eq!(ranges(&format!("text/csv;q={bad}"))[0].1, 0.0, "{bad}");
        }
        for (good, q) in [
            ("0", 0.0),
            ("1", 1.0),
            ("0.5", 0.5),
            ("1.000", 1.0),
            ("0.001", 0.001),
        ] {
            assert_eq!(ranges(&format!("text/csv;q={good}"))[0].1, q, "{good}");
        }
    }

    #[test]
    fn quoted_values_keep_their_commas_and_semicolons() {
        let all: Vec<_> = media_ranges(r#"a/b;p="x,y;z", c/d;p="\"q\",r""#).collect();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].param("p"), Some("x,y;z"));
        assert_eq!(all[1].essence, "c/d");
        assert_eq!(all[1].param("p"), Some(r#""q",r"#));
    }

    #[test]
    fn quoted_pairs_are_decoded() {
        let escaped = r#"application/ld+json;profile="http:\/\/www.w3.org/ns/json-ld#compacted""#;
        let plain = r#"application/ld+json;profile="http://www.w3.org/ns/json-ld#compacted""#;
        let bare = "application/ld+json;profile=http://www.w3.org/ns/json-ld#compacted";
        for accept in [escaped, plain, bare] {
            let r = media_ranges(accept).next().unwrap();
            assert_eq!(
                r.param("profile"),
                Some("http://www.w3.org/ns/json-ld#compacted"),
                "{accept}"
            );
        }
        assert_eq!(
            media_ranges(r#"a/b;p="x\\y""#).next().unwrap().param("p"),
            Some("x\\y")
        );
    }

    #[test]
    fn specificity_orders_exact_type_wildcard_and_any() {
        fn r(s: &str) -> MediaRange<'_> {
            media_ranges(s).next().unwrap()
        }
        assert_eq!(r("text/turtle").specificity("text/turtle"), Some(3));
        assert_eq!(r("text/*").specificity("text/turtle"), Some(2));
        assert_eq!(r("*/*").specificity("text/turtle"), Some(1));
        assert_eq!(r("text/*").specificity("application/json"), None);
        assert_eq!(r("text/csv").specificity("text/turtle"), None);
    }
}
