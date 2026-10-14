//! XML Schema regular expressions (XSD 1.1 Part 2, Appendix G) translated to a
//! `regex-syntax` [`Hir`] that matches the whole string.
//!
//! XSD patterns are implicitly anchored and have no anchors, back-references or
//! lazy quantifiers; `^` and `$` are ordinary characters. Every character class is
//! intersected with the XML `Char` production, since no other character can occur in a
//! string value. Block escapes (`\p{IsBasicLatin}`) are not implemented and report
//! [`PatternError::Unsupported`].

use regex_syntax::hir::{Class, ClassUnicode, ClassUnicodeRange, Hir, Repetition};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatternError {
    /// Not a regular expression in the XSD grammar.
    Invalid,
    /// Valid, but uses a construct this module does not implement.
    Unsupported,
}

/// Parses `pattern` into an [`Hir`] over the characters of a string value.
pub(crate) fn parse(pattern: &str) -> Result<Hir, PatternError> {
    let mut p = Parser {
        s: pattern.chars().collect(),
        i: 0,
    };
    let hir = p.reg_exp()?;
    if p.i != p.s.len() {
        return Err(PatternError::Invalid);
    }
    Ok(hir)
}

/// XML 1.0 `Char`: `#x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD] | [#x10000-#x10FFFF]`.
pub(crate) fn xml_char() -> ClassUnicode {
    ranges(&[
        ('\u{9}', '\u{A}'),
        ('\u{D}', '\u{D}'),
        ('\u{20}', '\u{D7FF}'),
        ('\u{E000}', '\u{FFFD}'),
        ('\u{10000}', '\u{10FFFF}'),
    ])
}

/// XML 1.0 (Fifth Edition) `NameStartChar`, the class `\i`.
pub(crate) fn name_start_char() -> ClassUnicode {
    ranges(&[
        (':', ':'),
        ('A', 'Z'),
        ('_', '_'),
        ('a', 'z'),
        ('\u{C0}', '\u{D6}'),
        ('\u{D8}', '\u{F6}'),
        ('\u{F8}', '\u{2FF}'),
        ('\u{370}', '\u{37D}'),
        ('\u{37F}', '\u{1FFF}'),
        ('\u{200C}', '\u{200D}'),
        ('\u{2070}', '\u{218F}'),
        ('\u{2C00}', '\u{2FEF}'),
        ('\u{3001}', '\u{D7FF}'),
        ('\u{F900}', '\u{FDCF}'),
        ('\u{FDF0}', '\u{FFFD}'),
        ('\u{10000}', '\u{EFFFF}'),
    ])
}

/// XML 1.0 (Fifth Edition) `NameChar`, the class `\c`.
pub(crate) fn name_char() -> ClassUnicode {
    let mut c = name_start_char();
    c.union(&ranges(&[
        ('-', '.'),
        ('0', '9'),
        ('\u{B7}', '\u{B7}'),
        ('\u{300}', '\u{36F}'),
        ('\u{203F}', '\u{2040}'),
    ]));
    c
}

fn ranges(rs: &[(char, char)]) -> ClassUnicode {
    ClassUnicode::new(rs.iter().map(|&(a, b)| ClassUnicodeRange::new(a, b)))
}

fn neg(mut c: ClassUnicode) -> ClassUnicode {
    c.negate();
    c
}

fn minus(mut a: ClassUnicode, b: &ClassUnicode) -> ClassUnicode {
    a.difference(b);
    a
}

/// `\p{name}` for an XSD general-category name.
fn category(name: &str) -> Result<ClassUnicode, PatternError> {
    const NAMES: &[&str] = &[
        "L", "Lu", "Ll", "Lt", "Lm", "Lo", "M", "Mn", "Mc", "Me", "N", "Nd", "Nl", "No", "P", "Pc",
        "Pd", "Ps", "Pe", "Pi", "Pf", "Po", "Z", "Zs", "Zl", "Zp", "S", "Sm", "Sc", "Sk", "So",
        "C", "Cc", "Cf", "Co", "Cn",
    ];
    if name.starts_with("Is") && name.len() > 2 {
        return Err(PatternError::Unsupported);
    }
    if !NAMES.contains(&name) {
        return Err(PatternError::Invalid);
    }
    let hir =
        regex_syntax::parse(&format!(r"\p{{{name}}}")).map_err(|_| PatternError::Unsupported)?;
    match hir.into_kind() {
        regex_syntax::hir::HirKind::Class(Class::Unicode(c)) => Ok(c),
        _ => Err(PatternError::Unsupported),
    }
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek() == Some(c);
        self.i += usize::from(hit);
        hit
    }

    fn next(&mut self) -> Result<char, PatternError> {
        let c = self.peek().ok_or(PatternError::Invalid)?;
        self.i += 1;
        Ok(c)
    }

    /// `regExp ::= branch ( '|' branch )*`
    fn reg_exp(&mut self) -> Result<Hir, PatternError> {
        let mut branches = vec![self.branch()?];
        while self.eat('|') {
            branches.push(self.branch()?);
        }
        Ok(Hir::alternation(branches))
    }

    /// `branch ::= piece*`, `piece ::= atom quantifier?`
    fn branch(&mut self) -> Result<Hir, PatternError> {
        let mut pieces = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            pieces.push(self.quantifier(atom)?);
        }
        Ok(Hir::concat(pieces))
    }

    fn quantifier(&mut self, atom: Hir) -> Result<Hir, PatternError> {
        let (min, max) = match self.peek() {
            Some('?') => (0, Some(1)),
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('{') => {
                self.i += 1;
                let min = self.number()?;
                let max = if self.eat(',') {
                    if self.peek() == Some('}') {
                        None
                    } else {
                        Some(self.number()?)
                    }
                } else {
                    Some(min)
                };
                if !self.eat('}') || max.is_some_and(|m| m < min) {
                    return Err(PatternError::Invalid);
                }
                return Ok(Hir::repetition(Repetition {
                    min,
                    max,
                    greedy: true,
                    sub: Box::new(atom),
                }));
            }
            _ => return Ok(atom),
        };
        self.i += 1;
        Ok(Hir::repetition(Repetition {
            min,
            max,
            greedy: true,
            sub: Box::new(atom),
        }))
    }

    fn number(&mut self) -> Result<u32, PatternError> {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        let digits: String = self.s[start..self.i].iter().collect();
        if digits.is_empty() {
            return Err(PatternError::Invalid);
        }
        // A bound past u32 cannot be compiled to an automaton anyway.
        digits.parse().map_err(|_| PatternError::Unsupported)
    }

    /// `atom ::= NormalChar | charClass | '(' regExp ')'`
    fn atom(&mut self) -> Result<Hir, PatternError> {
        let class = match self.next()? {
            '(' => {
                let inner = self.reg_exp()?;
                return if self.eat(')') {
                    Ok(inner)
                } else {
                    Err(PatternError::Invalid)
                };
            }
            '.' => neg(ranges(&[('\n', '\n'), ('\r', '\r')])),
            '[' => self.class_expr()?,
            '\\' => match self.escape()? {
                Esc::Char(c) => ranges(&[(c, c)]),
                Esc::Class(c) => c,
            },
            '?' | '*' | '+' | '{' | '}' | ')' | ']' | '|' => return Err(PatternError::Invalid),
            c => ranges(&[(c, c)]),
        };
        let mut class = class;
        class.intersect(&xml_char());
        Ok(Hir::class(Class::Unicode(class)))
    }

    /// After a `\`: a single-character escape or a class escape.
    fn escape(&mut self) -> Result<Esc, PatternError> {
        let c = self.next()?;
        Ok(match c {
            'n' => Esc::Char('\n'),
            'r' => Esc::Char('\r'),
            't' => Esc::Char('\t'),
            '\\' | '|' | '.' | '?' | '*' | '+' | '(' | ')' | '{' | '}' | '-' | '[' | ']' | '^' => {
                Esc::Char(c)
            }
            's' => Esc::Class(space()),
            'S' => Esc::Class(neg(space())),
            'i' => Esc::Class(name_start_char()),
            'I' => Esc::Class(neg(name_start_char())),
            'c' => Esc::Class(name_char()),
            'C' => Esc::Class(neg(name_char())),
            'd' => Esc::Class(category("Nd")?),
            'D' => Esc::Class(neg(category("Nd")?)),
            'w' => Esc::Class(word()?),
            'W' => Esc::Class(neg(word()?)),
            'p' | 'P' => {
                if !self.eat('{') {
                    return Err(PatternError::Invalid);
                }
                let start = self.i;
                while self.peek().is_some_and(|c| c != '}') {
                    self.i += 1;
                }
                let name: String = self.s[start..self.i].iter().collect();
                if !self.eat('}') {
                    return Err(PatternError::Invalid);
                }
                let class = category(&name)?;
                Esc::Class(if c == 'p' { class } else { neg(class) })
            }
            _ => return Err(PatternError::Invalid),
        })
    }

    /// After a `[`: `charGroup ']'`, where
    /// `charGroup ::= ( posCharGroup | '^' posCharGroup ) ( '-' charClassExpr )?`.
    fn class_expr(&mut self) -> Result<ClassUnicode, PatternError> {
        let negated = self.eat('^');
        let mut class = ClassUnicode::empty();
        let mut first = true;
        loop {
            match self.peek().ok_or(PatternError::Invalid)? {
                ']' if !first => {
                    self.i += 1;
                    break;
                }
                '-' if self.s.get(self.i + 1) == Some(&'[') && !first => {
                    self.i += 2;
                    let sub = self.class_expr()?;
                    if !self.eat(']') {
                        return Err(PatternError::Invalid);
                    }
                    let base = if negated { neg(class) } else { class };
                    return Ok(minus(base, &sub));
                }
                _ => {
                    let item = self.class_item()?;
                    class.union(&item);
                }
            }
            first = false;
        }
        Ok(if negated { neg(class) } else { class })
    }

    /// One `charRange` or `charClassEsc` inside a group.
    fn class_item(&mut self) -> Result<ClassUnicode, PatternError> {
        let lo = match self.next()? {
            '\\' => match self.escape()? {
                Esc::Char(c) => c,
                Esc::Class(c) => return Ok(c),
            },
            '[' => return Err(PatternError::Invalid),
            c => c,
        };
        // `lo-hi`, unless the `-` ends the group or starts a subtraction.
        if self.peek() == Some('-')
            && !matches!(self.s.get(self.i + 1), Some(']') | Some('[') | None)
        {
            self.i += 1;
            let hi = match self.next()? {
                '\\' => match self.escape()? {
                    Esc::Char(c) => c,
                    Esc::Class(_) => return Err(PatternError::Invalid),
                },
                '[' | ']' => return Err(PatternError::Invalid),
                c => c,
            };
            if hi < lo {
                return Err(PatternError::Invalid);
            }
            return Ok(ranges(&[(lo, hi)]));
        }
        Ok(ranges(&[(lo, lo)]))
    }
}

enum Esc {
    Char(char),
    Class(ClassUnicode),
}

/// `\s = [#x20\t\n\r]`
fn space() -> ClassUnicode {
    ranges(&[('\t', '\n'), ('\r', '\r'), (' ', ' ')])
}

/// `\w = [#x0000-#x10FFFF]-[\p{P}\p{Z}\p{C}]`
fn word() -> Result<ClassUnicode, PatternError> {
    let mut not_word = category("P")?;
    not_word.union(&category("Z")?);
    not_word.union(&category("C")?);
    Ok(neg(not_word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatypes::strings::Dfa;

    fn full(p: &str, s: &str) -> bool {
        Dfa::build(parse(p).unwrap()).unwrap().matches(s.as_bytes())
    }

    #[test]
    fn xsd_syntax_is_implicitly_anchored_and_literal_for_caret_dollar() {
        assert!(full("a+", "aaa") && !full("a+", "baaa"));
        assert!(full("^a$", "^a$") && !full("^a$", "a"));
        assert!(full("[a-c-[b]]+", "acca") && !full("[a-c-[b]]+", "abc"));
        assert!(full("[^a-z]", "Q") && !full("[^a-z]", "q"));
        assert!(full("[-a]", "-") && full("[a-]", "-"));
        assert!(full(r"\i\c*", "x:y-z.1") && !full(r"\i\c*", "1x"));
        assert!(full(r"\d{2,3}", "123") && !full(r"\d{2,3}", "1234"));
        assert!(full(r"\p{Lu}\P{Lu}", "Ab") && !full(r"\p{Lu}\P{Lu}", "AB"));
        assert!(full(".", "a") && !full(".", "\n"));
        assert!(full(r"(ab|c){2}", "abc"));
    }

    #[test]
    fn rejects_non_xsd_syntax_and_reports_blocks() {
        for bad in ["a{2,1}", "(a", "a)", "[a", r"\b", "*a", "[z-a]", r"\p{Foo}"] {
            assert_eq!(parse(bad).err(), Some(PatternError::Invalid), "{bad}");
        }
        assert_eq!(
            parse(r"\p{IsBasicLatin}").err(),
            Some(PatternError::Unsupported)
        );
    }
}
