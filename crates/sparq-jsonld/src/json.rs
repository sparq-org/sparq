//! The minimal, dependency-free JSON value model (`Json` AST).
//!
//! [OPUS-4.8] (sq-oy1f.23) Moved **verbatim** out of `sparq-engine`'s JSON-LD writer
//! (`serialize/compact.rs`) so the whole document-level JSON-LD 1.1 pipeline can share one
//! JSON value type without pulling in `serde_json`. The move is behaviour-neutral: the
//! engine re-exports this type at its old path (`sparq_engine::serialize::compact::Json`,
//! aliased `JsonLdValue`), so its writer and every downstream consumer keep compiling and
//! emitting byte-identical output.
//!
//! Object members preserve **insertion order** (the writer is order-deterministic), so an
//! object is a `Vec<(String, Json)>` rather than a hash map.

use std::fmt::Write as _;

/// A minimal JSON value used as the working representation for the JSON-LD pipeline. Object
/// members preserve insertion order so the emitted document is deterministic.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// A JSON string.
    Str(String),
    /// A pre-rendered JSON scalar token (`true`, `false`, a number). Stored verbatim so
    /// the writer's lossless native-coercion text is preserved byte-for-byte.
    Raw(String),
    /// A JSON array.
    Arr(Vec<Json>),
    /// A JSON object, members in insertion order.
    Obj(Vec<(String, Json)>),
}

impl Default for Json {
    /// The default JSON value is an empty object — the natural "no context" value for a
    /// processed active context's raw `@context`.
    fn default() -> Json {
        Json::Obj(Vec::new())
    }
}

impl Json {
    /// Constructs an empty JSON object.
    pub fn obj() -> Json {
        Json::Obj(Vec::new())
    }

    /// Inserts/overwrites a member, preserving first-insertion position for an existing key.
    /// A no-op on a non-object value.
    pub fn set(&mut self, key: &str, val: Json) {
        if let Json::Obj(members) = self {
            if let Some(slot) = members.iter_mut().find(|(k, _)| k == key) {
                slot.1 = val;
            } else {
                members.push((key.to_string(), val));
            }
        }
    }

    /// Borrows the value of member `key`, or `None` if this is not an object or the key is
    /// absent.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Borrows the inner string of a [`Json::Str`], or `None` for any other variant.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// True iff this is a [`Json::Obj`].
    pub fn is_obj(&self) -> bool {
        matches!(self, Json::Obj(_))
    }

    /// Serialises this value as canonical minified JSON into `out`.
    pub fn write(&self, out: &mut String) {
        match self {
            Json::Str(s) => {
                out.push('"');
                json_escape(s, out);
                out.push('"');
            }
            Json::Raw(r) => out.push_str(r),
            Json::Arr(items) => {
                out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    it.write(out);
                }
                out.push(']');
            }
            Json::Obj(members) => {
                out.push('{');
                for (i, (k, v)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    json_escape(k, out);
                    out.push_str("\":");
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    /// Parses a JSON text into a [`Json`] value. [OPUS-4.8] (sq-oy1f.24) A minimal,
    /// dependency-free recursive-descent parser used to read remote `@context` and
    /// `@import` documents retrieved through a [`DocumentLoader`](crate::loader::DocumentLoader).
    ///
    /// Numbers, `true`, `false`, and `null` are preserved verbatim as [`Json::Raw`] scalar
    /// tokens; strings are unescaped. Duplicate object keys keep the last value (first
    /// position). Object member order is preserved. Arrays and objects nested deeper than
    /// [`MAX_DEPTH`] are rejected, so the recursive walks over a parsed value stay within
    /// the stack.
    pub fn parse(input: &str) -> Result<Json, JsonParseError> {
        let mut p = Parser {
            bytes: input.as_bytes(),
            pos: 0,
            depth: 0,
        };
        p.skip_ws();
        let value = p.parse_value()?;
        p.skip_ws();
        if p.pos != p.bytes.len() {
            return Err(p.error("trailing characters after JSON value"));
        }
        Ok(value)
    }
}

/// An error from [`Json::parse`]: a human-readable message and the byte offset into the
/// input at which parsing failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonParseError {
    /// A human-readable description of the failure.
    pub message: String,
    /// The byte offset into the input at which the error was detected.
    pub position: usize,
}

impl std::fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON parse error at byte {}: {}", self.position, self.message)
    }
}

impl std::error::Error for JsonParseError {}

/// How deeply [`Json::parse`] lets arrays and objects nest.
pub const MAX_DEPTH: usize = 256;

/// A minimal recursive-descent JSON parser over the input bytes.
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// Arrays and objects open around the current position.
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, msg: &str) -> JsonParseError {
        JsonParseError {
            message: msg.to_string(),
            position: self.pos,
        }
    }

    fn skip_ws(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<Json, JsonParseError> {
        match self.peek() {
            Some(b'{' | b'[') => {
                if self.depth >= MAX_DEPTH {
                    return Err(self.error("arrays and objects nest too deeply"));
                }
                self.depth += 1;
                let value = if self.peek() == Some(b'{') {
                    self.parse_object()
                } else {
                    self.parse_array()
                };
                self.depth -= 1;
                value
            }
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", Json::Raw("true".to_string())),
            Some(b'f') => self.parse_literal("false", Json::Raw("false".to_string())),
            Some(b'n') => self.parse_literal("null", Json::Raw("null".to_string())),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            _ => Err(self.error("expected a JSON value")),
        }
    }

    fn parse_literal(&mut self, word: &str, value: Json) -> Result<Json, JsonParseError> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn parse_number(&mut self) -> Result<Json, JsonParseError> {
        // [OPUS-4.8] (sq-oy1f.24) Enforce the RFC 8259 number grammar strictly:
        //   number = [ "-" ] int [ frac ] [ exp ]
        //   int    = "0" / ( digit1-9 *DIGIT )   (no leading zeros)
        //   frac   = "." 1*DIGIT                 (a "." requires a digit)
        //   exp    = ("e" / "E") ["+" / "-"] 1*DIGIT
        // A minimal, remote-context parser must not silently accept malformed forms such as
        // `01`, `1.`, or `1e`, otherwise invalid JSON is treated as valid.
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        // Integer part: a lone "0", or a 1-9 lead followed by any digits.
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error("invalid number: expected a digit")),
        }
        // Fraction: "." then at least one digit.
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number: fraction requires a digit"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        // Exponent: e/E, optional sign, then at least one digit.
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number: exponent requires a digit"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let raw = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.error("invalid number"))?;
        Ok(Json::Raw(raw.to_string()))
    }

    fn parse_string(&mut self) -> Result<String, JsonParseError> {
        // Consumes the opening quote.
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1; // now at the escape character
                    match self.peek() {
                        Some(b'"') => self.push_simple_escape(&mut out, '"'),
                        Some(b'\\') => self.push_simple_escape(&mut out, '\\'),
                        Some(b'/') => self.push_simple_escape(&mut out, '/'),
                        Some(b'b') => self.push_simple_escape(&mut out, '\u{08}'),
                        Some(b'f') => self.push_simple_escape(&mut out, '\u{0C}'),
                        Some(b'n') => self.push_simple_escape(&mut out, '\n'),
                        Some(b'r') => self.push_simple_escape(&mut out, '\r'),
                        Some(b't') => self.push_simple_escape(&mut out, '\t'),
                        Some(b'u') => self.push_unicode_escape(&mut out)?,
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                // [OPUS-4.8] (sq-oy1f.24) RFC 8259 forbids raw control characters
                // (U+0000..=U+001F) inside a string; they must be escaped. Reject them so a
                // malformed remote `@context`/`@import` document is not treated as valid.
                Some(b) if b < 0x20 => {
                    return Err(self.error("unescaped control character in string"));
                }
                Some(_) => {
                    // [FABLE-5] (sq-hmd7l.42) Copy the maximal run of plain bytes (no
                    // quote, no backslash, no control character) in one step, validating
                    // UTF-8 once per run. The previous per-character path ran
                    // `str::from_utf8` over the WHOLE remaining input for every character
                    // — O(n²) on the document, and the dominant cost of the parse+expand
                    // pipeline on a ~10 KB document (measured with `perf`, work box,
                    // NON-canonical).
                    let start = self.pos;
                    while let Some(&b) = self.bytes.get(self.pos) {
                        if b == b'"' || b == b'\\' || b < 0x20 {
                            break;
                        }
                        self.pos += 1;
                    }
                    match std::str::from_utf8(&self.bytes[start..self.pos]) {
                        Ok(run) => out.push_str(run),
                        Err(e) => {
                            // Report the error at the first invalid byte — exactly where
                            // the old per-character path stopped.
                            self.pos = start + e.valid_up_to();
                            return Err(self.error("invalid UTF-8"));
                        }
                    }
                }
            }
        }
    }

    /// Pushes a single-character escape (`self.pos` is at the escape letter) and steps past it.
    fn push_simple_escape(&mut self, out: &mut String, c: char) {
        out.push(c);
        self.pos += 1;
    }

    /// Handles a `\u` escape (and a surrogate pair). `self.pos` is at `u`; on return it is
    /// positioned just past the last consumed hex digit.
    fn push_unicode_escape(&mut self, out: &mut String) -> Result<(), JsonParseError> {
        let cp = self.read_hex4_after_u()?;
        if (0xD800..=0xDBFF).contains(&cp) {
            // High surrogate: a low surrogate `\uXXXX` must follow.
            if self.bytes[self.pos..].starts_with(b"\\u") {
                self.pos += 1; // skip the '\', leaving pos at 'u'
                let lo = self.read_hex4_after_u()?;
                if !(0xDC00..=0xDFFF).contains(&lo) {
                    return Err(self.error("invalid low surrogate"));
                }
                let c = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                out.push(char::from_u32(c).ok_or_else(|| self.error("invalid code point"))?);
                Ok(())
            } else {
                Err(self.error("unpaired high surrogate"))
            }
        } else if (0xDC00..=0xDFFF).contains(&cp) {
            Err(self.error("unexpected low surrogate"))
        } else {
            out.push(char::from_u32(cp).ok_or_else(|| self.error("invalid code point"))?);
            Ok(())
        }
    }

    /// Reads the four hex digits of a `\u` escape. `self.pos` is at `u`; on return it is
    /// positioned just past the fourth hex digit.
    fn read_hex4_after_u(&mut self) -> Result<u32, JsonParseError> {
        let start = self.pos + 1;
        let end = start + 4;
        if end > self.bytes.len() {
            return Err(self.error("truncated \\u escape"));
        }
        let hex = std::str::from_utf8(&self.bytes[start..end])
            .map_err(|_| self.error("invalid \\u escape"))?;
        let cp = u32::from_str_radix(hex, 16).map_err(|_| self.error("invalid \\u escape"))?;
        self.pos = end;
        Ok(cp)
    }

    fn parse_array(&mut self) -> Result<Json, JsonParseError> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            self.skip_ws();
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn parse_object(&mut self) -> Result<Json, JsonParseError> {
        self.pos += 1; // '{'
        let mut obj = Json::obj();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(obj);
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a string key"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.error("expected ':'"));
            }
            self.pos += 1;
            self.skip_ws();
            let value = self.parse_value()?;
            obj.set(&key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(obj);
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }
}

/// Escapes a string as a JSON string body (without the surrounding quotes) per RFC 8259:
/// the two mandatory escapes (`"`, `\`), the short escapes for the common control chars,
/// and `\u00XX` for the remaining C0 controls. Everything else (including non-ASCII, which
/// JSON permits raw in UTF-8) passes through verbatim.
fn json_escape(s: &str, out: &mut String) {
    // Copy runs of bytes that need no escaping in one go; only `"`, `\\` and control
    // characters (all ASCII, so never inside a multi-byte sequence) are rewritten.
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let esc = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x08 => "\\b",
            0x0C => "\\f",
            b if b < 0x20 => "",
            _ => continue,
        };
        out.push_str(&s[start..i]);
        if esc.is_empty() {
            let _ = write!(out, "\\u{:04x}", b);
        } else {
            out.push_str(esc);
        }
        start = i + 1;
    }
    out.push_str(&s[start..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_empty_object() {
        assert_eq!(Json::default(), Json::Obj(Vec::new()));
        assert!(Json::default().is_obj());
    }

    #[test]
    fn obj_constructs_empty_object() {
        let o = Json::obj();
        assert_eq!(o, Json::Obj(Vec::new()));
        assert!(o.is_obj());
    }

    #[test]
    fn set_inserts_and_preserves_position_then_overwrites() {
        let mut o = Json::obj();
        o.set("a", Json::Str("1".into()));
        o.set("b", Json::Str("2".into()));
        o.set("a", Json::Str("3".into())); // overwrite keeps first position
        assert_eq!(
            o,
            Json::Obj(vec![
                ("a".into(), Json::Str("3".into())),
                ("b".into(), Json::Str("2".into())),
            ])
        );
        // set() on a non-object is a no-op.
        let mut arr = Json::Arr(vec![]);
        arr.set("x", Json::Raw("true".into()));
        assert_eq!(arr, Json::Arr(vec![]));
    }

    #[test]
    fn get_borrows_member_or_none() {
        let mut o = Json::obj();
        o.set("k", Json::Raw("42".into()));
        assert_eq!(o.get("k"), Some(&Json::Raw("42".into())));
        assert_eq!(o.get("missing"), None);
        // get() on a non-object is None.
        assert_eq!(Json::Str("x".into()).get("k"), None);
    }

    #[test]
    fn as_str_only_for_strings() {
        assert_eq!(Json::Str("hi".into()).as_str(), Some("hi"));
        assert_eq!(Json::Raw("1".into()).as_str(), None);
        assert_eq!(Json::obj().as_str(), None);
    }

    #[test]
    fn is_obj_discriminates() {
        assert!(Json::obj().is_obj());
        assert!(!Json::Arr(vec![]).is_obj());
        assert!(!Json::Str("x".into()).is_obj());
    }

    #[test]
    fn write_emits_canonical_minified_json_with_escaping() {
        let mut o = Json::obj();
        o.set("s", Json::Str("a\"b\\c\n\t".into()));
        o.set("n", Json::Raw("3.5".into()));
        o.set(
            "a",
            Json::Arr(vec![Json::Raw("true".into()), Json::Str("x".into())]),
        );
        let mut out = String::new();
        o.write(&mut out);
        assert_eq!(out, "{\"s\":\"a\\\"b\\\\c\\n\\t\",\"n\":3.5,\"a\":[true,\"x\"]}");
    }

    #[test]
    fn write_escapes_c0_controls_as_lower_hex() {
        let mut out = String::new();
        Json::Str("\u{01}\u{1f}".into()).write(&mut out);
        // A quote, the two C0 controls as `\u00XX` lower-hex, then a quote.
        assert_eq!(out, "\"\\u0001\\u001f\"");
    }

    #[test]
    fn parse_scalars_are_preserved_as_raw_tokens() {
        assert_eq!(Json::parse("true").unwrap(), Json::Raw("true".into()));
        assert_eq!(Json::parse("false").unwrap(), Json::Raw("false".into()));
        assert_eq!(Json::parse("null").unwrap(), Json::Raw("null".into()));
        assert_eq!(Json::parse("  42 ").unwrap(), Json::Raw("42".into()));
        assert_eq!(Json::parse("-3.5e10").unwrap(), Json::Raw("-3.5e10".into()));
        assert_eq!(Json::parse(r#""hi""#).unwrap(), Json::Str("hi".into()));
    }

    #[test]
    fn parse_object_preserves_order_and_last_duplicate_wins() {
        let v = Json::parse(r#"{"b": 1, "a": "x", "b": 2}"#).unwrap();
        assert_eq!(
            v,
            Json::Obj(vec![
                ("b".into(), Json::Raw("2".into())),
                ("a".into(), Json::Str("x".into())),
            ])
        );
    }

    #[test]
    fn parse_nested_arrays_and_objects() {
        let v = Json::parse(r#"{"@context": {"n": "x"}, "list": [1, "two", true, null]}"#).unwrap();
        let ctx = v.get("@context").unwrap();
        assert_eq!(ctx.get("n"), Some(&Json::Str("x".into())));
        let list = v.get("list").unwrap();
        assert_eq!(
            list,
            &Json::Arr(vec![
                Json::Raw("1".into()),
                Json::Str("two".into()),
                Json::Raw("true".into()),
                Json::Raw("null".into()),
            ])
        );
        assert_eq!(Json::parse("[]").unwrap(), Json::Arr(vec![]));
        assert_eq!(Json::parse("{}").unwrap(), Json::obj());
    }

    #[test]
    fn parse_decodes_string_escapes_including_unicode_and_surrogates() {
        // Two-char escapes: quote, backslash, newline, tab, and `\/`.
        let input = "\"a\\\"b\\\\c\\n\\t\\/A\"";
        assert_eq!(Json::parse(input).unwrap(), Json::Str("a\"b\\c\n\t/A".into()));
        // A BMP `\u` escape (U+00E9 "é").
        assert_eq!(Json::parse("\"\\u00e9\"").unwrap(), Json::Str("é".into()));
        // A surrogate-pair escape `😀` encoding U+1F600 ("😀").
        assert_eq!(
            Json::parse("\"\\uD83D\\uDE00\"").unwrap(),
            Json::Str("\u{1F600}".into())
        );
        // A lone (unpaired) high surrogate is rejected.
        assert!(Json::parse("\"\\uD83D\"").is_err());
    }

    /// [FABLE-5] (sq-hmd7l.42) The run-copying string fast path: plain runs (including
    /// multi-byte UTF-8) interleaved with escapes and terminators must decode exactly as
    /// the old per-character path did, and invalid UTF-8 must still be reported at the
    /// first invalid byte.
    #[test]
    fn parse_string_run_fast_path_preserves_semantics() {
        // A long plain ASCII run (exercises the run copy, not the per-char path).
        let long = "x".repeat(2048);
        assert_eq!(
            Json::parse(&format!("\"{long}\"")).unwrap(),
            Json::Str(long.clone())
        );
        // Multi-byte UTF-8 inside a run, runs split by escapes on both sides.
        assert_eq!(
            Json::parse("\"héllo wörld\\n后半 run🚀\"").unwrap(),
            Json::Str("héllo wörld\n后半 run🚀".into())
        );
        // A run terminated by a control character still errors (the run must stop there),
        // and the error position points at the offending byte just past the run — the
        // run loop must not swallow or misattribute the terminator. (`Json::parse` takes
        // `&str`, and the run loop only breaks at ASCII bytes — never mid-UTF-8-sequence
        // — so the defensive invalid-UTF-8 arm is unreachable from the public API.)
        let e = Json::parse("\"abcd\u{01}\"").unwrap_err();
        assert_eq!(e.position, 5, "error must point at the offending byte after the run");
        // An unterminated long run still reports "unterminated string", not a panic/hang.
        assert!(Json::parse(&format!("\"{long}")).is_err());
    }

    #[test]
    fn parse_rejects_malformed_input() {
        assert!(Json::parse("{").is_err());
        assert!(Json::parse("[1,]").is_err());
        assert!(Json::parse(r#"{"k": }"#).is_err());
        assert!(Json::parse("nul").is_err());
        assert!(Json::parse("1 2").is_err()); // trailing characters
        assert!(Json::parse(r#""unterminated"#).is_err());
        let err = Json::parse("@").unwrap_err();
        assert!(err.to_string().contains("byte 0"));
    }

    #[test]
    fn parse_rejects_unescaped_control_char_in_string() {
        // RFC 8259 forbids raw control chars (U+0000..=U+001F) inside a string.
        assert!(Json::parse("\"a\u{01}b\"").is_err());
        assert!(Json::parse("\"tab\there\"").is_err()); // literal TAB
        assert!(Json::parse("\"nl\nhere\"").is_err()); // literal newline
        // The escaped forms remain accepted.
        assert_eq!(Json::parse("\"\\t\"").unwrap(), Json::Str("\t".into()));
    }

    #[test]
    fn parse_rejects_malformed_numbers() {
        // Leading zeros are forbidden (int = "0" / digit1-9 *DIGIT).
        assert!(Json::parse("01").is_err());
        assert!(Json::parse("-01").is_err());
        assert!(Json::parse("00").is_err());
        // A fraction requires at least one digit.
        assert!(Json::parse("1.").is_err());
        assert!(Json::parse("1.e5").is_err());
        // An exponent requires at least one digit.
        assert!(Json::parse("1e").is_err());
        assert!(Json::parse("1e+").is_err());
        assert!(Json::parse("1E-").is_err());
        // A lone sign / no integer part is invalid.
        assert!(Json::parse("-").is_err());
        assert!(Json::parse(".5").is_err());
        // Well-formed forms still parse (including a bare "0").
        assert_eq!(Json::parse("0").unwrap(), Json::Raw("0".into()));
        assert_eq!(Json::parse("-0.5e-2").unwrap(), Json::Raw("-0.5e-2".into()));
        assert_eq!(Json::parse("10").unwrap(), Json::Raw("10".into()));
    }
}
