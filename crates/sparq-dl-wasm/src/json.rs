//! Hand-written JSON, so the bundle carries no serde: a string escaper for the report and a
//! small strict parser for the options object.

use sparq_reason_dl::owl2dl::Budget;
use std::fmt::Write as _;

/// Appends `s` to `out` as a JSON string literal: `"` and `\` are escaped, and every
/// control character below U+0020 is written as `\u00XX`. Everything else (including
/// non-ASCII) is copied as UTF-8, which JSON allows.
pub(crate) fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Appends a JSON array of strings.
pub(crate) fn push_str_array<'a>(out: &mut String, items: impl IntoIterator<Item = &'a str>) {
    out.push('[');
    for (i, s) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_str(out, s);
    }
    out.push(']');
}

/// A parsed JSON value. Numbers keep their lexical form; the caller decides what it accepts.
#[derive(Debug, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// The deepest nesting the parser accepts (the options object needs two levels).
const MAX_DEPTH: usize = 32;

/// Parses one JSON document (RFC 8259), rejecting trailing content.
pub(crate) fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("trailing content after the JSON value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        format!("invalid options JSON at byte {}: {what}", self.i)
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.i) {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.s.get(self.i) == Some(&b) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn keyword(&mut self, word: &str, v: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("unexpected token"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        self.ws();
        match self.s.get(self.i) {
            None => Err(self.err("unexpected end of input")),
            Some(b'n') => self.keyword("null", Value::Null),
            Some(b't') => self.keyword("true", Value::Bool(true)),
            Some(b'f') => self.keyword("false", Value::Bool(false)),
            Some(b'"') => self.string().map(Value::Str),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat(b']') {
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b']') {
                        return Ok(Value::Array(items));
                    }
                    if !self.eat(b',') {
                        return Err(self.err("expected ',' or ']'"));
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut members = Vec::new();
                self.ws();
                if self.eat(b'}') {
                    return Ok(Value::Object(members));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(self.err("expected a member name"));
                    }
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(b':') {
                        return Err(self.err("expected ':'"));
                    }
                    members.push((key, self.value(depth + 1)?));
                    self.ws();
                    if self.eat(b'}') {
                        return Ok(Value::Object(members));
                    }
                    if !self.eat(b',') {
                        return Err(self.err("expected ',' or '}'"));
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while let Some(b'0'..=b'9') = self.s.get(self.i) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        self.eat(b'-');
        if self.eat(b'0') {
            if let Some(b'0'..=b'9') = self.s.get(self.i) {
                return Err(self.err("leading zero in a number"));
            }
        } else if self.digits() == 0 {
            return Err(self.err("expected a digit"));
        }
        if self.eat(b'.') && self.digits() == 0 {
            return Err(self.err("expected a fraction digit"));
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if self.digits() == 0 {
                return Err(self.err("expected an exponent digit"));
            }
        }
        // The lexeme is ASCII by construction.
        let lexeme = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        Ok(Value::Number(lexeme))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let hex = self
            .s
            .get(self.i..self.i + 4)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .filter(|_| self.s[self.i..self.i + 4].iter().all(u8::is_ascii_hexdigit))
            .ok_or_else(|| self.err("expected four hex digits"))?;
        self.i += 4;
        Ok(hex)
    }

    fn string(&mut self) -> Result<String, String> {
        // Caller has checked the opening quote.
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            match self.s.get(self.i) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') => {
                    self.i += 1;
                    let esc = *self
                        .s
                        .get(self.i)
                        .ok_or_else(|| self.err("unterminated escape"))?;
                    self.i += 1;
                    let c = match esc {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if !(self.eat(b'\\') && self.eat(b'u')) {
                                    return Err(self.err("unpaired surrogate"));
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("unpaired surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            char::from_u32(code).ok_or_else(|| self.err("unpaired surrogate"))?
                        }
                        _ => return Err(self.err("invalid escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                Some(&b) if b < 0x20 => return Err(self.err("unescaped control character")),
                Some(&b) => {
                    out.push(b);
                    self.i += 1;
                }
            }
        }
        // The input is a `&str` and escapes add whole characters, so this is valid UTF-8.
        String::from_utf8(out).map_err(|_| self.err("invalid UTF-8"))
    }
}

/// Reads a non-negative integer budget limit (`null` keeps the default).
fn limit(name: &str, v: &Value) -> Result<Option<u64>, String> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => n
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("options.budget.{name} is too large")),
        _ => Err(format!(
            "options.budget.{name} must be a non-negative integer"
        )),
    }
}

/// The reasoner budget the options ask for. `options` is `null`/absent or an object whose
/// only member is `budget`, itself an object with the optional members `maxSteps` and
/// `maxNodes` (non-negative integers). Unknown members are an error, so a misspelt limit
/// is never silently ignored.
pub(crate) fn budget_from_options(options: Option<&str>) -> Result<Budget, String> {
    let mut budget = Budget::default();
    let Some(text) = options else {
        return Ok(budget);
    };
    let members = match parse(text)? {
        Value::Null => return Ok(budget),
        Value::Object(m) => m,
        _ => return Err("options must be a JSON object".into()),
    };
    for (key, v) in &members {
        match (key.as_str(), v) {
            ("budget", Value::Null) => {}
            ("budget", Value::Object(limits)) => {
                for (name, v) in limits {
                    match name.as_str() {
                        "maxSteps" => {
                            if let Some(n) = limit(name, v)? {
                                budget.max_steps = n;
                            }
                        }
                        "maxNodes" => {
                            if let Some(n) = limit(name, v)? {
                                budget.max_nodes = usize::try_from(n)
                                    .map_err(|_| "options.budget.maxNodes is too large")?;
                            }
                        }
                        other => {
                            return Err(format!(
                                "unknown option budget.{other} (expected maxSteps or maxNodes)"
                            ))
                        }
                    }
                }
            }
            ("budget", _) => return Err("options.budget must be an object".into()),
            (other, _) => return Err(format!("unknown option {other:?} (expected budget)")),
        }
    }
    Ok(budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc(s: &str) -> String {
        let mut out = String::new();
        push_str(&mut out, s);
        out
    }

    #[test]
    fn escapes_quotes_backslashes_and_control_characters() {
        assert_eq!(esc(""), r#""""#);
        assert_eq!(esc(r#"a"b"#), r#""a\"b""#);
        assert_eq!(esc(r"a\b"), r#""a\\b""#);
        assert_eq!(esc("\n\r\t"), r#""\u000a\u000d\u0009""#);
        assert_eq!(esc("\u{0}\u{1f}"), r#""\u0000\u001f""#);
        // Not control characters: copied verbatim.
        assert_eq!(esc(" \u{7f}é𝄞/"), "\" \u{7f}é𝄞/\"");
    }

    #[test]
    fn escaped_strings_parse_back_to_the_original() {
        let hostile = "\"\\\n\u{0}\u{1}\u{1f} <x> \"lit\"@en é 𝄞 \u{2028}";
        assert_eq!(parse(&esc(hostile)).unwrap(), Value::Str(hostile.into()));
        let mut out = String::new();
        push_str_array(&mut out, ["a", "b\"c"]);
        assert_eq!(
            parse(&out).unwrap(),
            Value::Array(vec![Value::Str("a".into()), Value::Str("b\"c".into())])
        );
    }

    #[test]
    fn parser_reads_json_and_rejects_malformed_input() {
        assert_eq!(
            parse(r#" { "a" : [1, -2.5e+3, true, false, null, "\u00e9\ud834\udd1e\/"] } "#)
                .unwrap(),
            Value::Object(vec![(
                "a".into(),
                Value::Array(vec![
                    Value::Number("1".into()),
                    Value::Number("-2.5e+3".into()),
                    Value::Bool(true),
                    Value::Bool(false),
                    Value::Null,
                    Value::Str("é𝄞/".into()),
                ])
            )])
        );
        for bad in [
            "",
            "{",
            "{\"a\" 1}",
            "{\"a\":1,}",
            "[1 2]",
            "01",
            "1.",
            "1e",
            "-",
            "\"\\x\"",
            "\"\\ud834\"",
            "\"\\udd1e\"",
            "\"a\nb\"",
            "\"abc",
            "nul",
            "{} {}",
            "{a:1}",
        ] {
            assert!(parse(bad).is_err(), "accepted {bad:?}");
        }
        let deep = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn budget_options() {
        let d = Budget::default();
        assert_eq!(budget_from_options(None).unwrap(), d);
        assert_eq!(budget_from_options(Some("null")).unwrap(), d);
        assert_eq!(budget_from_options(Some("{}")).unwrap(), d);
        assert_eq!(budget_from_options(Some(r#"{"budget":null}"#)).unwrap(), d);
        let b = budget_from_options(Some(r#"{"budget":{"maxSteps":10,"maxNodes":7}}"#)).unwrap();
        assert_eq!((b.max_steps, b.max_nodes), (10, 7));
        let b = budget_from_options(Some(r#"{"budget":{"maxSteps":5,"maxNodes":null}}"#)).unwrap();
        assert_eq!((b.max_steps, b.max_nodes), (5, d.max_nodes));
        for bad in [
            "[]",
            "1",
            r#"{"budgte":{}}"#,
            r#"{"budget":{"maxStep":1}}"#,
            r#"{"budget":{"maxSteps":-1}}"#,
            r#"{"budget":{"maxSteps":1.5}}"#,
            r#"{"budget":{"maxSteps":1e3}}"#,
            r#"{"budget":{"maxSteps":"10"}}"#,
            r#"{"budget":{"maxSteps":99999999999999999999999}}"#,
            r#"{"budget":3}"#,
        ] {
            assert!(budget_from_options(Some(bad)).is_err(), "accepted {bad}");
        }
    }
}
