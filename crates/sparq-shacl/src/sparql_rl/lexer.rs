//! Longest-match terminals from SPARQL-RL grammar productions 127–161.
use super::{error, ErrorKind, Result};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Iri,
    PName,
    Blank,
    Var,
    ShortString,
    LongString,
    Number,
    Lang,
    Word,
    Symbol,
}

#[derive(Debug)]
pub(super) struct Token<'a> {
    pub text: &'a str,
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

fn terminals() -> &'static [(Kind, Regex)] {
    static TERMINALS: OnceLock<Vec<(Kind, Regex)>> = OnceLock::new();
    TERMINALS.get_or_init(|| {
        let base = r"A-Za-z\x{C0}-\x{D6}\x{D8}-\x{F6}\x{F8}-\x{2FF}\x{370}-\x{37D}\x{37F}-\x{1FFF}\x{200C}-\x{200D}\x{2070}-\x{218F}\x{2C00}-\x{2FEF}\x{3001}-\x{D7FF}\x{F900}-\x{FDCF}\x{FDF0}-\x{FFFD}\x{10000}-\x{EFFFF}";
        let u = format!("{base}_");
        let chars = format!(r"{u}\-0-9\x{{B7}}\x{{300}}-\x{{36F}}\x{{203F}}-\x{{2040}}");
        let plx = r"(?:%[0-9A-Fa-f]{2}|\\[_~.\-!$&'()*+,;=/?#@%])";
        let ns = format!("(?:[{base}](?:[{chars}.]*[{chars}])?)?:");
        let local = format!("(?:[{u}:0-9]|{plx})(?:(?:[{chars}.:]|{plx})*(?:[{chars}:]|{plx}))?");
        let uchar = r"(?:\\u[0-9A-Fa-f]{4}|\\U[0-9A-Fa-f]{8})";
        let escape = format!(r#"(?:\\[tbnrf\"'\\]|{uchar})"#);
        let patterns = vec![
            (Kind::Iri, format!(r#"<(?:[^<>"{{}}|^`\\\x00-\x20]|{uchar})*>"#)),
            (Kind::PName, format!("{ns}(?:{local})?")),
            (Kind::Blank, format!("_:[{u}0-9](?:[{chars}.]*[{chars}])?")),
            (Kind::Var, format!(r"[?$][{u}0-9][{u}0-9\x{{B7}}\x{{300}}-\x{{36F}}\x{{203F}}-\x{{2040}}]*")),
            (Kind::LongString, format!(r#"(?:'''(?:(?:'|'')?(?:[^'\\]|{escape}))*'''|"""(?:(?:"|"")?(?:[^"\\]|{escape}))*""")"#)),
            (Kind::ShortString, format!(r#"(?:'(?:[^'\\\r\n]|{escape})*'|"(?:[^"\\\r\n]|{escape})*")"#)),
            (Kind::Number, r"[+-]?(?:(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)[eE][+-]?[0-9]+|[0-9]*\.[0-9]+|[0-9]+)".into()),
            (Kind::Lang, r"@[a-zA-Z]+(?:-[a-zA-Z0-9]+)*(?:--[a-zA-Z]+)?".into()),
            (Kind::Word, r"[A-Za-z_][A-Za-z_0-9]*".into()),
            (Kind::Symbol, r"(?:<<\(|\)>>|<<|>>|\{\||\|\}|:=|\^\^|\|\||&&|!=|<=|>=|[{}()\[\].;,~^/!+*=<>-])".into()),
        ];
        patterns.into_iter().map(|(kind, pattern)| {
            (kind, Regex::new(&format!("^(?:{pattern})")).expect("SPARQL-RL terminal regex"))
        }).collect()
    })
}

pub(super) fn lex(source: &str) -> Result<Vec<Token<'_>>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < source.len() {
        let rest = &source[pos..];
        let ch = rest.chars().next().unwrap();
        if matches!(ch, ' ' | '\t' | '\r' | '\n') {
            pos += 1;
            continue;
        }
        if ch == '#' {
            pos += rest.find(['\r', '\n']).unwrap_or(rest.len());
            continue;
        }
        let mut best = None;
        for (kind, regex) in terminals() {
            if let Some(m) = regex.find(rest) {
                if best.is_none_or(|(_, len)| m.end() > len) {
                    best = Some((*kind, m.end()));
                }
            }
        }
        let Some((kind, len)) = best else {
            return Err(error(
                source,
                pos,
                ErrorKind::Syntax,
                "Invalid SPARQL-RL token",
            ));
        };
        let text = &rest[..len];
        if matches!(kind, Kind::Iri | Kind::ShortString | Kind::LongString) {
            check_escapes(source, text, pos)?;
        }
        out.push(Token {
            text,
            kind,
            start: pos,
            end: pos + len,
        });
        pos += len;
    }
    Ok(out)
}

fn check_escapes(source: &str, text: &str, offset: usize) -> Result<()> {
    let mut chars = text.char_indices();
    while let Some((pos, ch)) = chars.next() {
        if ch != '\\' {
            continue;
        }
        let Some((_, escape)) = chars.next() else {
            break;
        };
        let len = match escape {
            'u' => 4,
            'U' => 8,
            _ => continue,
        };
        let digits: String = chars.by_ref().take(len).map(|(_, ch)| ch).collect();
        let scalar = u32::from_str_radix(&digits, 16)
            .ok()
            .and_then(char::from_u32);
        if scalar.is_none() {
            return Err(error(
                source,
                offset + pos,
                ErrorKind::Syntax,
                "Unicode escape is not a scalar value",
            ));
        }
    }
    Ok(())
}
