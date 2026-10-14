//! The string-valued spaces: `rdf:PlainLiteral` (plain strings and language-tagged
//! pairs) and `xsd:anyURI`.
//!
//! A set of string values is a boolean formula over [`Atom`]s, each a regular
//! language over an encoding of the value: a plain string is its UTF-8 bytes and a pair
//! `<s, tag>` is `s`, a NUL byte, then the lowercase tag (NUL is not an XML `Char`, so the
//! encoding is unambiguous). Emptiness is a breadth-first search of the product of the
//! atoms' automata, which decides any boolean combination of patterns, lengths, language
//! ranges and enumerated values exactly. A search past [`STATE_BUDGET`] answers Unknown.

use super::xsdre::{self, xml_char, PatternError};
use super::Tri;
use regex_automata::dfa::{dense, Automaton, StartKind};
use regex_automata::util::primitives::StateID;
use regex_automata::util::start;
use regex_automata::{Anchored, MatchKind};
use regex_syntax::hir::{Class, ClassUnicode, ClassUnicodeRange, Hir, Look, Repetition};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

/// Product states explored before an emptiness check gives up with Unknown.
const STATE_BUDGET: usize = 100_000;
/// Bytes one compiled automaton may occupy.
const DFA_SIZE_LIMIT: usize = 16 << 20;

/// The value-space subsets of `xsd:string` in the datatype map, each defined by the
/// XSD pattern of its lexical space (XSD 1.1 Part 2 §3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StrKind {
    String,
    NormalizedString,
    Token,
    Language,
    Name,
    NCName,
    NmToken,
    /// `xsd:anyURI`: any string (relative references are parsed as they are).
    AnyUri,
}

impl StrKind {
    fn pattern(self) -> &'static str {
        match self {
            StrKind::String | StrKind::AnyUri => r"[\s\S]*",
            StrKind::NormalizedString => r"[^\t\n\r]*",
            StrKind::Token => r"([^\s]+( [^\s]+)*)?",
            StrKind::Language => "[a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*",
            StrKind::Name => r"\i\c*",
            StrKind::NCName => r"[\i-[:]][\c-[:]]*",
            StrKind::NmToken => r"\c+",
        }
    }
}

/// A regular set of encoded string values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Atom {
    /// A plain string in the value space of this datatype.
    Plain(StrKind),
    /// The string part matches this XSD pattern (already validated).
    Pattern(String),
    /// The string part has between `lo` and `hi` characters.
    Len(u64, Option<u64>),
    /// A language-tagged pair whose tag matches this lowercase basic language range
    /// (`*` matches every tag).
    Lang(String),
    /// Exactly this encoded value.
    Is(Vec<u8>),
}

/// A boolean combination of atoms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum F {
    Const(bool),
    Atom(Atom),
    Not(Box<F>),
    And(Vec<F>),
    Or(Vec<F>),
}

impl F {
    pub(crate) fn atom(a: Atom) -> F {
        F::Atom(a)
    }

    pub(crate) fn not(self) -> F {
        match self {
            F::Const(b) => F::Const(!b),
            F::Not(f) => *f,
            f => F::Not(Box::new(f)),
        }
    }

    pub(crate) fn and(self, o: F) -> F {
        match (self, o) {
            (F::Const(false), _) | (_, F::Const(false)) => F::Const(false),
            (F::Const(true), f) | (f, F::Const(true)) => f,
            (F::And(mut a), F::And(b)) => {
                a.extend(b);
                F::And(a)
            }
            (F::And(mut a), f) | (f, F::And(mut a)) => {
                a.push(f);
                F::And(a)
            }
            (a, b) => F::And(vec![a, b]),
        }
    }

    pub(crate) fn or(self, o: F) -> F {
        match (self, o) {
            (F::Const(true), _) | (_, F::Const(true)) => F::Const(true),
            (F::Const(false), f) | (f, F::Const(false)) => f,
            (F::Or(mut a), F::Or(b)) => {
                a.extend(b);
                F::Or(a)
            }
            (F::Or(mut a), f) | (f, F::Or(mut a)) => {
                a.push(f);
                F::Or(a)
            }
            (a, b) => F::Or(vec![a, b]),
        }
    }

    fn eval(&self, atom: &mut impl FnMut(&Atom) -> bool) -> bool {
        match self {
            F::Const(b) => *b,
            F::Atom(a) => atom(a),
            F::Not(f) => !f.eval(atom),
            F::And(fs) => fs.iter().all(|f| f.eval(atom)),
            F::Or(fs) => fs.iter().any(|f| f.eval(atom)),
        }
    }

    fn atoms<'a>(&'a self, out: &mut Vec<&'a Atom>) {
        match self {
            F::Const(_) => {}
            F::Atom(a) => {
                if !out.contains(&a) {
                    out.push(a);
                }
            }
            F::Not(f) => f.atoms(out),
            F::And(fs) | F::Or(fs) => fs.iter().for_each(|f| f.atoms(out)),
        }
    }
}

/// Encodes a plain string or a language-tagged pair (the tag already lowercase).
pub(crate) fn encode(s: &str, lang: Option<&str>) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    if let Some(l) = lang {
        v.push(0);
        v.extend_from_slice(l.as_bytes());
    }
    v
}

/// Whether `s` is a plain string of datatype `kind` (characters in its value space).
pub(crate) fn is_kind(kind: StrKind, s: &str) -> Tri {
    match dfa(&Atom::Plain(kind)) {
        Ok(d) => Tri::from(d.matches(s.as_bytes())),
        Err(_) => Tri::Unknown,
    }
}

/// Whether `pattern` is an XSD regular expression this module can evaluate.
pub(crate) fn pattern_status(pattern: &str) -> Tri {
    match xsdre::parse(pattern) {
        Ok(_) => Tri::True,
        Err(PatternError::Invalid) => Tri::False,
        Err(PatternError::Unsupported) => Tri::Unknown,
    }
}

/// Whether `f` holds for the encoded value `v`.
pub(crate) fn holds(f: &F, v: &[u8]) -> Tri {
    let mut unknown = false;
    let r = f.eval(&mut |a| match atom_holds(a, v) {
        Some(b) => b,
        None => {
            unknown = true;
            false
        }
    });
    if unknown {
        Tri::Unknown
    } else {
        Tri::from(r)
    }
}

fn atom_holds(a: &Atom, v: &[u8]) -> Option<bool> {
    let text = v.split(|&b| b == 0).next().unwrap_or(&[]);
    Some(match a {
        Atom::Len(lo, hi) => {
            let n = chars(text);
            n >= *lo && hi.is_none_or(|h| n <= h)
        }
        Atom::Is(w) => v == w.as_slice(),
        _ => dfa(a).ok()?.matches(v),
    })
}

fn chars(bytes: &[u8]) -> u64 {
    bytes.iter().filter(|&&b| b & 0xC0 != 0x80).count() as u64
}

/// Whether `f`, restricted to the space (`with_pairs` = rdf:PlainLiteral's strings and
/// pairs; otherwise the plain strings of `xsd:anyURI`), denotes no value.
pub(crate) fn is_empty(f: &F, with_pairs: bool) -> Tri {
    if let F::Const(b) = f {
        return Tri::from(!b);
    }
    let mut atoms = Vec::new();
    f.atoms(&mut atoms);
    let universe = match universe(with_pairs) {
        Ok(u) => u,
        Err(_) => return Tri::Unknown,
    };
    let mut autos = vec![Auto::Dfa(universe)];
    for a in &atoms {
        autos.push(match a {
            Atom::Len(lo, hi) => Auto::Len {
                lo: *lo,
                hi: *hi,
                cap: hi.map_or(*lo, |h| h + 1),
            },
            Atom::Is(w) => Auto::Is(w.clone()),
            _ => match dfa(a) {
                Ok(d) => Auto::Dfa(d),
                Err(_) => return Tri::Unknown,
            },
        });
    }
    let reps = representative_bytes(&autos);
    let start: Option<Vec<u32>> = autos.iter().map(Auto::start).collect();
    let Some(start) = start else {
        return Tri::Unknown;
    };
    let mut seen = HashSet::from([start.clone()]);
    let mut queue = VecDeque::from([start]);
    while let Some(state) = queue.pop_front() {
        if autos[0].accepts(state[0]) {
            let index = |a: &Atom| atoms.iter().position(|b| *b == a).expect("collected") + 1;
            if f.eval(&mut |a| autos[index(a)].accepts(state[index(a)])) {
                return Tri::False;
            }
        }
        for &b in &reps {
            let next: Vec<u32> = autos
                .iter()
                .zip(&state)
                .map(|(a, &s)| a.next(s, b))
                .collect();
            if autos[0].is_dead(next[0]) || seen.contains(&next) {
                continue;
            }
            if seen.len() >= STATE_BUDGET {
                return Tri::Unknown;
            }
            seen.insert(next.clone());
            queue.push_back(next);
        }
    }
    Tri::True
}

/// One byte from each class of bytes every automaton treats alike.
fn representative_bytes(autos: &[Auto]) -> Vec<u8> {
    let mut classes: HashMap<Vec<u16>, u8> = HashMap::new();
    for b in 0..=255u8 {
        let key = autos
            .iter()
            .map(|a| match a {
                Auto::Dfa(d) => u16::from(d.0.byte_classes().get(b)),
                Auto::Len { .. } => u16::from(b == 0) * 2 + u16::from(b & 0xC0 == 0x80),
                Auto::Is(w) => w.iter().position(|&x| x == b).map_or(0, |i| i as u16 + 1),
            })
            .collect();
        classes.entry(key).or_insert(b);
    }
    let mut reps: Vec<u8> = classes.into_values().collect();
    reps.sort_unstable();
    reps
}

enum Auto {
    Dfa(Rc<Dfa>),
    /// Characters of the string part so far, saturating at `cap`; bit 31 = past the NUL.
    Len {
        lo: u64,
        hi: Option<u64>,
        cap: u64,
    },
    /// Bytes matched so far; `u32::MAX` = mismatch.
    Is(Vec<u8>),
}

const PAST_NUL: u32 = 1 << 31;

impl Auto {
    fn start(&self) -> Option<u32> {
        match self {
            Auto::Dfa(d) => Some(d.start()),
            Auto::Len { cap, .. } => (*cap < u64::from(PAST_NUL)).then_some(0),
            Auto::Is(_) => Some(0),
        }
    }

    fn next(&self, s: u32, b: u8) -> u32 {
        match self {
            Auto::Dfa(d) => {
                d.0.next_state(StateID::new_unchecked(s as usize), b)
                    .as_u32()
            }
            Auto::Len { cap, .. } => {
                if s & PAST_NUL != 0 || b == 0 {
                    s | PAST_NUL
                } else if b & 0xC0 != 0x80 {
                    (u64::from(s) + 1).min(*cap) as u32
                } else {
                    s
                }
            }
            Auto::Is(w) => match w.get(s as usize) {
                Some(&x) if s != u32::MAX && x == b => s + 1,
                _ => u32::MAX,
            },
        }
    }

    fn accepts(&self, s: u32) -> bool {
        match self {
            Auto::Dfa(d) => d.accepts(s),
            Auto::Len { lo, hi, .. } => {
                let n = u64::from(s & !PAST_NUL);
                n >= *lo && hi.is_none_or(|h| n <= h)
            }
            Auto::Is(w) => s as usize == w.len(),
        }
    }

    fn is_dead(&self, s: u32) -> bool {
        match self {
            Auto::Dfa(d) => d.0.is_dead_state(StateID::new_unchecked(s as usize)),
            _ => false,
        }
    }
}

/// An anchored DFA that accepts exactly the byte strings matching its regex in full.
pub(crate) struct Dfa(dense::DFA<Vec<u32>>);

impl Dfa {
    pub(crate) fn build(hir: Hir) -> Result<Dfa, ()> {
        let hir = Hir::concat(vec![hir, Hir::look(Look::End)]);
        let nfa = regex_automata::nfa::thompson::Compiler::new()
            .configure(
                regex_automata::nfa::thompson::Config::new().nfa_size_limit(Some(DFA_SIZE_LIMIT)),
            )
            .build_from_hir(&hir)
            .map_err(|_| ())?;
        let dfa = dense::Builder::new()
            .configure(
                dense::Config::new()
                    .match_kind(MatchKind::All)
                    .start_kind(StartKind::Anchored)
                    .dfa_size_limit(Some(DFA_SIZE_LIMIT))
                    .determinize_size_limit(Some(DFA_SIZE_LIMIT)),
            )
            .build_from_nfa(&nfa)
            .map_err(|_| ())?;
        Ok(Dfa(dfa))
    }

    fn start(&self) -> u32 {
        self.0
            .start_state(&start::Config::new().anchored(Anchored::Yes))
            .expect("anchored starts are built")
            .as_u32()
    }

    fn accepts(&self, s: u32) -> bool {
        let eoi = self.0.next_eoi_state(StateID::new_unchecked(s as usize));
        self.0.is_match_state(eoi)
    }

    pub(crate) fn matches(&self, bytes: &[u8]) -> bool {
        let mut s = self.start();
        for &b in bytes {
            s = self
                .0
                .next_state(StateID::new_unchecked(s as usize), b)
                .as_u32();
        }
        self.accepts(s)
    }
}

thread_local! {
    /// Compiled automata by atom (and the two universes under pseudo-atoms), so a
    /// datatype's DFA is built once per thread.
    static DFAS: RefCell<HashMap<Atom, Rc<Dfa>>> = RefCell::new(HashMap::new());
}

fn cached(key: Atom, build: impl FnOnce() -> Result<Hir, ()>) -> Result<Rc<Dfa>, ()> {
    if let Some(d) = DFAS.with(|m| m.borrow().get(&key).cloned()) {
        return Ok(d);
    }
    let d = Rc::new(Dfa::build(build()?)?);
    DFAS.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() >= 256 {
            m.clear();
        }
        m.insert(key, Rc::clone(&d));
    });
    Ok(d)
}

fn class(c: ClassUnicode) -> Hir {
    Hir::class(Class::Unicode(c))
}

fn star(h: Hir) -> Hir {
    Hir::repetition(Repetition {
        min: 0,
        max: None,
        greedy: true,
        sub: Box::new(h),
    })
}

fn opt(h: Hir) -> Hir {
    Hir::repetition(Repetition {
        min: 0,
        max: Some(1),
        greedy: true,
        sub: Box::new(h),
    })
}

fn any_text() -> Hir {
    star(class(xml_char()))
}

/// `[a-z]{1,8}(-[a-z0-9]{1,8})*`: a lowercase language tag.
fn lang_tag() -> Hir {
    let sub = |r: &[(char, char)]| {
        Hir::repetition(Repetition {
            min: 1,
            max: Some(8),
            greedy: true,
            sub: Box::new(class(ClassUnicode::new(
                r.iter().map(|&(a, b)| ClassUnicodeRange::new(a, b)),
            ))),
        })
    };
    Hir::concat(vec![
        sub(&[('a', 'z')]),
        star(Hir::concat(vec![
            Hir::literal(*b"-"),
            sub(&[('0', '9'), ('a', 'z')]),
        ])),
    ])
}

fn universe(with_pairs: bool) -> Result<Rc<Dfa>, ()> {
    cached(Atom::Pattern(format!("\0universe{with_pairs}")), || {
        let pairs = Hir::concat(vec![Hir::literal(*b"\0"), lang_tag()]);
        Ok(if with_pairs {
            Hir::concat(vec![any_text(), opt(pairs)])
        } else {
            any_text()
        })
    })
}

fn dfa(a: &Atom) -> Result<Rc<Dfa>, ()> {
    cached(a.clone(), || {
        Ok(match a {
            Atom::Plain(k) => xsdre::parse(k.pattern()).map_err(|_| ())?,
            Atom::Pattern(p) => {
                let any_suffix = opt(Hir::concat(vec![
                    Hir::literal(*b"\0"),
                    star(Hir::dot(regex_syntax::hir::Dot::AnyChar)),
                ]));
                Hir::concat(vec![xsdre::parse(p).map_err(|_| ())?, any_suffix])
            }
            Atom::Lang(r) => {
                let tag = if r == "*" {
                    lang_tag()
                } else {
                    let rest = Hir::concat(vec![
                        Hir::literal(*b"-"),
                        star(Hir::dot(regex_syntax::hir::Dot::AnyChar)),
                    ]);
                    Hir::concat(vec![Hir::literal(r.as_bytes().to_vec()), opt(rest)])
                };
                Hir::concat(vec![any_text(), Hir::literal(*b"\0"), tag])
            }
            Atom::Len(..) | Atom::Is(_) => return Err(()),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(p: &str) -> F {
        F::atom(Atom::Pattern(p.into()))
    }

    #[test]
    fn product_search_decides_boolean_combinations() {
        let s = F::atom(Atom::Plain(StrKind::String));
        assert_eq!(
            is_empty(
                &s.clone().and(pat("a+")).and(F::atom(Atom::Len(0, Some(0)))),
                true
            ),
            Tri::True
        );
        assert_eq!(
            is_empty(
                &s.clone().and(pat("a+")).and(F::atom(Atom::Len(3, Some(3)))),
                true
            ),
            Tri::False
        );
        // a{1,2} minus {"a", "aa"} is empty.
        let pair = pat("a{1,2}").and(
            F::atom(Atom::Is(b"a".to_vec()))
                .or(F::atom(Atom::Is(b"aa".to_vec())))
                .not(),
        );
        assert_eq!(is_empty(&s.clone().and(pair), true), Tri::True);
        // NCName ∩ pattern ".*:.*" is empty; Name ∩ it is not.
        let colon = pat(".*:.*");
        assert_eq!(
            is_empty(
                &F::atom(Atom::Plain(StrKind::NCName)).and(colon.clone()),
                true
            ),
            Tri::True
        );
        assert_eq!(
            is_empty(&F::atom(Atom::Plain(StrKind::Name)).and(colon), true),
            Tri::False
        );
        // Every tagged pair has a tag: "*" ∪ plain strings covers the whole space.
        let all = F::atom(Atom::Lang("*".into())).or(s.clone());
        assert_eq!(is_empty(&all.not(), true), Tri::True);
        assert_eq!(
            is_empty(&F::atom(Atom::Lang("en".into())).and(s), true),
            Tri::True
        );
    }

    #[test]
    fn membership_runs_each_atom_on_the_encoding() {
        let en = F::atom(Atom::Lang("en".into()));
        assert_eq!(holds(&en, &encode("x", Some("en-gb"))), Tri::True);
        assert_eq!(holds(&en, &encode("x", Some("eng"))), Tri::False);
        assert_eq!(holds(&en, &encode("x", None)), Tri::False);
        let len = F::atom(Atom::Len(2, Some(2)));
        assert_eq!(holds(&len, &encode("é€", Some("fr"))), Tri::True);
        assert_eq!(is_kind(StrKind::Token, " a"), Tri::False);
        assert_eq!(is_kind(StrKind::Token, "a b"), Tri::True);
        assert_eq!(is_kind(StrKind::String, "a\u{1}"), Tri::False);
        assert_eq!(is_kind(StrKind::Name, "é:x"), Tri::True);
    }
}
