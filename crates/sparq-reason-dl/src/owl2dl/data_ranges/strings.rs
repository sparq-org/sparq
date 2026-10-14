//! The string-like value spaces: strings with and without a language tag, `xsd:anyURI`
//! and `rdf:XMLLiteral`.
//!
//! A set of such values is a boolean formula over [`Atom`]s, each a regular language
//! over an encoding of the value: a string is its UTF-8 bytes, and a tagged string
//! `<s, tag>` is `s`, the byte `0xFF`, then the lowercase tag (`0xFF` never occurs in
//! UTF-8, so the encoding is unambiguous). Emptiness and counting walk the product of the
//! atoms' automata, which decides any boolean combination of patterns, lengths, language
//! ranges and enumerated values exactly. A walk past [`STATE_BUDGET`] states gives up.

use super::xsdre::{self, PatternError};
use regex_automata::dfa::{dense, Automaton, StartKind};
use regex_automata::util::primitives::StateID;
use regex_automata::util::start;
use regex_automata::{Anchored, MatchKind};
use regex_syntax::hir::{Class, ClassUnicode, ClassUnicodeRange, Dot, Hir, Look, Repetition};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// Product states explored before a check gives up.
const STATE_BUDGET: usize = 100_000;
/// Bytes one compiled automaton may occupy.
const DFA_SIZE_LIMIT: usize = 16 << 20;
/// The byte between a string and its language tag.
const SEP: u8 = 0xFF;

/// The string datatypes whose value spaces are proper subsets of all strings, each as
/// the XSD pattern of exactly the values `datatypes::in_value_space` admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StrKind {
    NormalizedString,
    Token,
    Language,
    Name,
    NcName,
    NmToken,
}

impl StrKind {
    fn pattern(self) -> &'static str {
        match self {
            StrKind::NormalizedString => r"[^\t\n\r]*",
            StrKind::Token => r"([^\s]+( [^\s]+)*)?",
            StrKind::Language => "[a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*",
            StrKind::Name => r"\i\c*",
            StrKind::NcName => r"[\i-[:]][\c-[:]]*",
            StrKind::NmToken => r"\c+",
        }
    }
}

/// Which string-like space a formula ranges over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Space {
    /// Strings, and strings with a language tag.
    Text,
    /// `xsd:anyURI` values.
    Uri,
    /// `rdf:XMLLiteral` values, by lexical form.
    Xml,
}

/// A regular set of encoded values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Atom {
    /// An untagged string in this datatype's value space.
    Plain(StrKind),
    /// Any untagged string.
    Untagged,
    /// The string part matches this XSD pattern (already validated).
    Pattern(String),
    /// The string part has between `lo` and `hi` characters.
    Len(u64, Option<u64>),
    /// A tagged string whose tag matches this extended language range (RFC 4647 §3.3.2,
    /// already validated and lowercase; `*` matches every tag).
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

/// Encodes a string, or a tagged string (the tag already lowercase).
pub(crate) fn encode(s: &str, tag: Option<&str>) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    if let Some(t) = tag {
        v.push(SEP);
        v.extend_from_slice(t.as_bytes());
    }
    v
}

/// Whether `pattern` is an XSD regular expression this module can evaluate: `Some(true)`
/// yes, `Some(false)` not an XSD regular expression, `None` valid but not implemented.
pub(crate) fn pattern_status(pattern: &str) -> Option<bool> {
    match xsdre::parse(pattern) {
        Ok(_) => Some(true),
        Err(PatternError::Invalid) => Some(false),
        Err(PatternError::Unsupported) => None,
    }
}

/// Whether `range` is an extended language range (RFC 4647 §2.2).
pub(crate) fn is_language_range(range: &str) -> bool {
    let subtag = |p: &str, alpha: bool| {
        !p.is_empty()
            && p.len() <= 8
            && p.bytes().all(|b| {
                if alpha {
                    b.is_ascii_alphabetic()
                } else {
                    b.is_ascii_alphanumeric()
                }
            })
    };
    let mut parts = range.split('-');
    let first = parts.next().unwrap_or("");
    (first == "*" || subtag(first, true)) && parts.all(|p| p == "*" || subtag(p, false))
}

/// Whether `f` holds for the encoded value `v`; `None` when an automaton could not be built.
pub(crate) fn holds(f: &F, v: &[u8]) -> Option<bool> {
    let mut failed = false;
    let r = f.eval(&mut |a| {
        atom_holds(a, v).unwrap_or_else(|| {
            failed = true;
            false
        })
    });
    (!failed).then_some(r)
}

fn atom_holds(a: &Atom, v: &[u8]) -> Option<bool> {
    let text = v.split(|&b| b == SEP).next().unwrap_or(&[]);
    Some(match a {
        Atom::Len(lo, hi) => {
            let n = chars(text);
            n >= *lo && hi.is_none_or(|h| n <= h)
        }
        Atom::Is(w) => v == w.as_slice(),
        Atom::Untagged => !v.contains(&SEP),
        _ => dfa(a).ok()?.matches(v),
    })
}

fn chars(bytes: &[u8]) -> u64 {
    bytes.iter().filter(|&&b| b & 0xC0 != 0x80).count() as u64
}

/// How many values of `space` satisfy `f`, exact up to `need` and saturating from there
/// (`u128::MAX` also stands for infinitely many). `None` = the automata or the walk
/// outgrew their budgets.
pub(crate) fn count(f: &F, space: Space, need: u128) -> Option<u128> {
    if let F::Const(false) = f {
        return Some(0);
    }
    let mut atoms = Vec::new();
    f.atoms(&mut atoms);
    let mut autos = vec![Auto::Dfa(universe(space).ok()?)];
    for a in &atoms {
        autos.push(match a {
            Atom::Len(lo, hi) => Auto::Len {
                lo: *lo,
                hi: *hi,
                cap: hi.map_or(*lo, |h| h + 1),
            },
            Atom::Is(w) => Auto::Is(w.clone()),
            Atom::Untagged => Auto::Untagged,
            _ => Auto::Dfa(dfa(a).ok()?),
        });
    }
    let classes = byte_classes(&autos);
    let start: Vec<u32> = autos.iter().map(Auto::start).collect::<Option<_>>()?;
    let accepting = |state: &[u32]| {
        autos[0].accepts(state[0]) && {
            let idx = |a: &Atom| atoms.iter().position(|b| *b == a).expect("collected") + 1;
            f.eval(&mut |a| autos[idx(a)].accepts(state[idx(a)]))
        }
    };

    // Explore the product, recording each edge with how many bytes take it.
    let mut index: HashMap<Vec<u32>, usize> = HashMap::from([(start.clone(), 0)]);
    let mut states = vec![start];
    let mut edges: Vec<Vec<(usize, u128)>> = Vec::new();
    let mut queue = VecDeque::from([0usize]);
    while let Some(i) = queue.pop_front() {
        if need <= 1 && accepting(&states[i]) {
            return Some(1);
        }
        let mut out = Vec::new();
        for &(b, mult) in &classes {
            let next: Vec<u32> = autos
                .iter()
                .zip(&states[i])
                .map(|(a, &s)| a.next(s, b))
                .collect();
            if autos[0].is_dead(next[0]) {
                continue;
            }
            let j = match index.get(&next) {
                Some(&j) => j,
                None => {
                    if states.len() >= STATE_BUDGET {
                        return None;
                    }
                    index.insert(next.clone(), states.len());
                    states.push(next);
                    queue.push_back(states.len() - 1);
                    states.len() - 1
                }
            };
            out.push((j, mult));
        }
        edges.resize(edges.len().max(i + 1), Vec::new());
        edges[i] = out;
    }
    if need <= 1 {
        return Some(0);
    }
    edges.resize(states.len(), Vec::new());
    let acc: Vec<bool> = states.iter().map(|s| accepting(s)).collect();

    // Live states reach an accepting one. A cycle through live states gives infinitely
    // many values; otherwise the live part is a DAG whose accepted paths are counted.
    let mut rev: Vec<Vec<usize>> = vec![Vec::new(); states.len()];
    for (i, out) in edges.iter().enumerate() {
        for &(j, _) in out {
            rev[j].push(i);
        }
    }
    let mut live = acc.clone();
    let mut stack: Vec<usize> = (0..states.len()).filter(|&i| acc[i]).collect();
    while let Some(j) = stack.pop() {
        for &i in &rev[j] {
            if !live[i] {
                live[i] = true;
                stack.push(i);
            }
        }
    }
    if !live[0] {
        return Some(0);
    }
    // Kahn's algorithm over the live subgraph; nodes it cannot order lie on a cycle.
    let mut indeg = vec![0usize; states.len()];
    for i in (0..states.len()).filter(|&i| live[i]) {
        for &(j, _) in &edges[i] {
            indeg[j] += usize::from(live[j]);
        }
    }
    let mut order = Vec::new();
    let mut ready: Vec<usize> = (0..states.len())
        .filter(|&i| live[i] && indeg[i] == 0)
        .collect();
    while let Some(i) = ready.pop() {
        order.push(i);
        for &(j, _) in &edges[i] {
            if live[j] {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    ready.push(j);
                }
            }
        }
    }
    if order.len() < live.iter().filter(|&&l| l).count() {
        return Some(u128::MAX);
    }
    let mut ways = vec![0u128; states.len()];
    for &i in order.iter().rev() {
        let mut n = u128::from(acc[i]);
        for &(j, mult) in &edges[i] {
            n = n.saturating_add(mult.saturating_mul(ways[j]));
        }
        ways[i] = n;
    }
    Some(ways[0])
}

/// One byte from each class of bytes every automaton treats alike, with the class size.
fn byte_classes(autos: &[Auto]) -> Vec<(u8, u128)> {
    let mut classes: HashMap<Vec<u16>, (u8, u128)> = HashMap::new();
    for b in 0..=255u8 {
        let key = autos
            .iter()
            .map(|a| match a {
                Auto::Dfa(d) => u16::from(d.0.byte_classes().get(b)),
                Auto::Len { .. } => u16::from(b == SEP) * 2 + u16::from(b & 0xC0 == 0x80),
                Auto::Untagged => u16::from(b == SEP),
                Auto::Is(w) => w.iter().position(|&x| x == b).map_or(0, |i| i as u16 + 1),
            })
            .collect();
        classes.entry(key).or_insert((b, 0)).1 += 1;
    }
    let mut out: Vec<(u8, u128)> = classes.into_values().collect();
    out.sort_unstable();
    out
}

enum Auto {
    Dfa(Rc<Dfa>),
    /// Characters of the string part so far, saturating at `cap`; bit 31 = past the tag
    /// separator.
    Len {
        lo: u64,
        hi: Option<u64>,
        cap: u64,
    },
    /// 0 = no separator seen, 1 = seen.
    Untagged,
    /// Bytes matched so far; `u32::MAX` = mismatch.
    Is(Vec<u8>),
}

const TAGGED: u32 = 1 << 31;

impl Auto {
    fn start(&self) -> Option<u32> {
        match self {
            Auto::Dfa(d) => Some(d.start()),
            Auto::Len { cap, .. } => (*cap < u64::from(TAGGED)).then_some(0),
            Auto::Untagged | Auto::Is(_) => Some(0),
        }
    }

    fn next(&self, s: u32, b: u8) -> u32 {
        match self {
            Auto::Dfa(d) => d.next(s, b),
            Auto::Len { cap, .. } => {
                if s & TAGGED != 0 || b == SEP {
                    s | TAGGED
                } else if b & 0xC0 != 0x80 {
                    (u64::from(s) + 1).min(*cap) as u32
                } else {
                    s
                }
            }
            Auto::Untagged => s | u32::from(b == SEP),
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
                let n = u64::from(s & !TAGGED);
                n >= *lo && hi.is_none_or(|h| n <= h)
            }
            Auto::Untagged => s == 0,
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

    fn next(&self, s: u32, b: u8) -> u32 {
        self.0
            .next_state(StateID::new_unchecked(s as usize), b)
            .as_u32()
    }

    fn accepts(&self, s: u32) -> bool {
        let eoi = self.0.next_eoi_state(StateID::new_unchecked(s as usize));
        self.0.is_match_state(eoi)
    }

    pub(crate) fn matches(&self, bytes: &[u8]) -> bool {
        self.accepts(bytes.iter().fold(self.start(), |s, &b| self.next(s, b)))
    }
}

/// What a compiled automaton is for: an atom, or the universe of a space.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Atom(Atom),
    Universe(Space),
}

thread_local! {
    /// Compiled automata, so each pattern and datatype is built once per thread.
    static DFAS: RefCell<HashMap<Key, Rc<Dfa>>> = RefCell::new(HashMap::new());
}

fn cached(key: Key, build: impl FnOnce() -> Result<Hir, ()>) -> Result<Rc<Dfa>, ()> {
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

fn class(rs: &[(char, char)]) -> Hir {
    Hir::class(Class::Unicode(ClassUnicode::new(
        rs.iter().map(|&(a, b)| ClassUnicodeRange::new(a, b)),
    )))
}

fn rep(h: Hir, min: u32, max: Option<u32>) -> Hir {
    Hir::repetition(Repetition {
        min,
        max,
        greedy: true,
        sub: Box::new(h),
    })
}

fn any_text() -> Hir {
    rep(Hir::dot(Dot::AnyChar), 0, None)
}

fn sep() -> Hir {
    Hir::literal(vec![SEP])
}

/// A subtag `[a-z0-9]{min,8}` (`alpha`: letters only).
fn subtag(min: u32, alpha: bool) -> Hir {
    let rs: &[(char, char)] = if alpha {
        &[('a', 'z')]
    } else {
        &[('0', '9'), ('a', 'z')]
    };
    rep(class(rs), min, Some(8))
}

fn dash_then(h: Hir) -> Hir {
    Hir::concat(vec![Hir::literal(*b"-"), h])
}

/// `[a-z]{1,8}(-[a-z0-9]{1,8})*`: a lowercase language tag.
fn lang_tag() -> Hir {
    Hir::concat(vec![
        subtag(1, true),
        rep(dash_then(subtag(1, false)), 0, None),
    ])
}

/// The tags an extended language range matches (RFC 4647 §3.3.2): the first subtag must
/// match; each later non-wildcard subtag must match some later tag subtag, skipping only
/// subtags that are not singletons; wildcards match anything.
fn lang_range(range: &str) -> Hir {
    let mut parts = range.split('-');
    let first = parts.next().unwrap_or("*");
    let mut seq = vec![if first == "*" {
        subtag(1, true)
    } else {
        Hir::literal(first.as_bytes().to_vec())
    }];
    for p in parts.filter(|p| *p != "*") {
        seq.push(rep(dash_then(subtag(2, false)), 0, None));
        seq.push(Hir::literal(format!("-{p}").into_bytes()));
    }
    seq.push(rep(dash_then(subtag(1, false)), 0, None));
    Hir::concat(seq)
}

fn universe(space: Space) -> Result<Rc<Dfa>, ()> {
    cached(Key::Universe(space), || {
        Ok(match space {
            Space::Text => Hir::concat(vec![
                any_text(),
                rep(Hir::concat(vec![sep(), lang_tag()]), 0, Some(1)),
            ]),
            Space::Uri | Space::Xml => any_text(),
        })
    })
}

fn dfa(a: &Atom) -> Result<Rc<Dfa>, ()> {
    cached(Key::Atom(a.clone()), || {
        let tail = rep(Hir::concat(vec![sep(), any_text()]), 0, Some(1));
        Ok(match a {
            Atom::Plain(k) => xsdre::parse(k.pattern()).map_err(|_| ())?,
            Atom::Pattern(p) => Hir::concat(vec![xsdre::parse(p).map_err(|_| ())?, tail]),
            Atom::Lang(r) => Hir::concat(vec![any_text(), sep(), lang_range(r)]),
            Atom::Len(..) | Atom::Is(_) | Atom::Untagged => return Err(()),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(p: &str) -> F {
        F::atom(Atom::Pattern(p.into()))
    }

    fn plain() -> F {
        F::atom(Atom::Untagged)
    }

    fn all(f: &F) -> Option<u128> {
        count(f, Space::Text, u128::MAX)
    }

    #[test]
    fn product_walk_decides_and_counts_boolean_combinations() {
        assert_eq!(
            all(&plain().and(pat("a+")).and(F::atom(Atom::Len(0, Some(0))))),
            Some(0)
        );
        assert_eq!(
            all(&plain().and(pat("a+")).and(F::atom(Atom::Len(3, Some(3))))),
            Some(1)
        );
        assert_eq!(all(&plain().and(pat("[ab]{2}"))), Some(4));
        assert_eq!(all(&plain().and(pat("a*"))), Some(u128::MAX));
        let both = F::atom(Atom::Is(b"a".to_vec())).or(F::atom(Atom::Is(b"aa".to_vec())));
        assert_eq!(all(&plain().and(pat("a{1,2}")).and(both.not())), Some(0));
        let colon = pat(".*:.*");
        assert_eq!(
            all(&F::atom(Atom::Plain(StrKind::NcName)).and(colon.clone())),
            Some(0)
        );
        assert_eq!(
            count(
                &F::atom(Atom::Plain(StrKind::Name)).and(colon),
                Space::Text,
                1
            ),
            Some(1)
        );
        // Untagged strings and every tag together make up the whole space.
        assert_eq!(
            all(&F::atom(Atom::Lang("*".into())).or(plain()).not()),
            Some(0)
        );
        assert_eq!(all(&F::atom(Atom::Lang("en".into())).and(plain())), Some(0));
    }

    #[test]
    fn counts_multiply_byte_classes() {
        // One character: every Unicode scalar value.
        let scalars = 0x11_0000 - 0x800;
        assert_eq!(
            all(&plain().and(F::atom(Atom::Len(1, Some(1))))),
            Some(scalars)
        );
        assert_eq!(
            count(&F::atom(Atom::Len(1, Some(1))), Space::Uri, u128::MAX),
            Some(scalars)
        );
        assert_eq!(
            all(&plain()
                .and(pat("[a-c]"))
                .and(F::atom(Atom::Is(b"b".to_vec())).not())),
            Some(2)
        );
    }

    #[test]
    fn extended_ranges_skip_non_singleton_subtags() {
        let m = |r: &str, tag: &str| {
            holds(&F::atom(Atom::Lang(r.into())), &encode("x", Some(tag))).unwrap()
        };
        assert!(m("de-de", "de-de") && m("de-de", "de-de-1996"));
        assert!(
            m("de-de", "de-latn-de"),
            "extended filtering skips the script subtag"
        );
        assert!(!m("de-de", "de-x-de"), "but never a singleton");
        assert!(m("de-*-de", "de-latn-de") && m("*-de", "en-de") && m("*", "fr"));
        assert!(!m("de", "deu") && !m("de-de", "de"));
        assert!(!holds(&F::atom(Atom::Lang("*".into())), &encode("x", None)).unwrap());
        assert!(
            is_language_range("de-*-DE") && is_language_range("*") && !is_language_range("de--de")
        );
    }

    #[test]
    fn membership_runs_each_atom_on_the_encoding() {
        let len = F::atom(Atom::Len(2, Some(2)));
        assert_eq!(holds(&len, &encode("é€", Some("fr"))), Some(true));
        let token = F::atom(Atom::Plain(StrKind::Token));
        assert_eq!(holds(&token, b" a"), Some(false));
        assert_eq!(holds(&token, b"a b"), Some(true));
        assert_eq!(
            holds(&F::atom(Atom::Plain(StrKind::Name)), "é:x".as_bytes()),
            Some(true)
        );
    }
}
