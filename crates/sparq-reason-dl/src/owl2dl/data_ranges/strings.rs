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
/// Product-state components (states times automata) and edges one check may record.
const CELL_BUDGET: usize = 2 << 20;
/// Bytes one compiled automaton may occupy.
const DFA_SIZE_LIMIT: usize = 16 << 20;
/// Bytes all the automata of one check, and all cached automata, may occupy.
const TOTAL_DFA_BYTES: usize = 64 << 20;
/// Longer patterns answer Unknown, bounding what parsing allocates before the automaton
/// limits apply.
const MAX_PATTERN_BYTES: usize = 4096;
/// Formulas with more nodes answer Unknown, bounding what indexing allocates.
const MAX_FORMULA_NODES: usize = 1 << 16;
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
    /// A tagged string whose tag matches this basic language range (RFC 4647 §3.3.1,
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

    /// Whether the formula has at most `*budget` nodes (spending the budget as it counts).
    fn within(&self, budget: &mut usize) -> bool {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        match self {
            F::Const(_) | F::Atom(_) => true,
            F::Not(f) => f.within(budget),
            F::And(fs) | F::Or(fs) => fs.iter().all(|f| f.within(budget)),
        }
    }

    /// The formula over atom indices, collecting each distinct atom once into `atoms`.
    fn index<'a>(&'a self, atoms: &mut Vec<&'a Atom>, seen: &mut HashMap<&'a Atom, usize>) -> G {
        match self {
            F::Const(b) => G::Const(*b),
            F::Atom(a) => G::Atom(*seen.entry(a).or_insert_with(|| {
                atoms.push(a);
                atoms.len() - 1
            })),
            F::Not(f) => G::Not(Box::new(f.index(atoms, seen))),
            F::And(fs) => G::And(fs.iter().map(|f| f.index(atoms, seen)).collect()),
            F::Or(fs) => G::Or(fs.iter().map(|f| f.index(atoms, seen)).collect()),
        }
    }
}

/// A formula whose atoms are indices into a list.
enum G {
    Const(bool),
    Atom(usize),
    Not(Box<G>),
    And(Vec<G>),
    Or(Vec<G>),
}

impl G {
    fn eval(&self, atom: &impl Fn(usize) -> bool) -> bool {
        match self {
            G::Const(b) => *b,
            G::Atom(i) => atom(*i),
            G::Not(g) => !g.eval(atom),
            G::And(gs) => gs.iter().all(|g| g.eval(atom)),
            G::Or(gs) => gs.iter().any(|g| g.eval(atom)),
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
    if pattern.len() > MAX_PATTERN_BYTES {
        return None;
    }
    match xsdre::parse(pattern) {
        Ok(_) => Some(true),
        Err(PatternError::Invalid) => Some(false),
        Err(PatternError::Unsupported) => None,
    }
}

/// Whether `range` is a basic language range (RFC 4647 §2.1).
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
    range == "*" || (subtag(parts.next().unwrap_or(""), true) && parts.all(|p| subtag(p, false)))
}

/// Whether the encoded value `v` is in `space`; `None` when its automaton could not be built.
pub(crate) fn in_space(space: Space, v: &[u8]) -> Option<bool> {
    Some(universe(space).ok()?.matches(v))
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
    match (f, space) {
        (F::Const(false), _) => return Some(0),
        // Infinitely many XML literals are well-balanced (any text without markup), but
        // which strings are, and how they canonicalise, is not checked: only the whole
        // space is counted.
        (F::Const(true), Space::Xml) => return Some(u128::MAX),
        (_, Space::Xml) => return None,
        _ => {}
    }
    let mut budget = MAX_FORMULA_NODES;
    if !f.within(&mut budget) {
        return None;
    }
    let mut atoms = Vec::new();
    let g = f.index(&mut atoms, &mut HashMap::new());
    // One automaton per atom, except that every enumerated value shares one trie: an atom
    // holds when its automaton accepts, or when the trie ends at the value's node.
    let mut autos = vec![Auto::Dfa(universe(space).ok()?)];
    // Every automaton, and the trie, counts against one byte budget as it is added.
    let mut bytes = autos[0].bytes();
    let mut trie = Trie::default();
    let mut tests = Vec::with_capacity(atoms.len());
    for a in &atoms {
        let auto = match a {
            Atom::Is(w) => {
                bytes = bytes.saturating_add(w.len().saturating_mul(TRIE_NODE_BYTES));
                if bytes > TOTAL_DFA_BYTES {
                    return None;
                }
                tests.push(Test::Ends(trie.insert(w)));
                continue;
            }
            Atom::Len(lo, hi) => Auto::Len {
                lo: *lo,
                hi: *hi,
                cap: match hi {
                    Some(h) => h.checked_add(1)?,
                    None => *lo,
                },
            },
            Atom::Untagged => Auto::Untagged,
            _ => Auto::Dfa(dfa(a).ok()?),
        };
        bytes = bytes.saturating_add(auto.bytes());
        if bytes > TOTAL_DFA_BYTES {
            return None;
        }
        tests.push(Test::Accepts(autos.len()));
        autos.push(auto);
    }
    let at_trie = autos.len();
    autos.push(Auto::Trie(Box::new(trie)));
    let classes = byte_classes(&autos);
    let start: Vec<u32> = autos.iter().map(Auto::start).collect::<Option<_>>()?;
    let accepting = |state: &[u32]| {
        autos[0].accepts(state[0])
            && g.eval(&|i| match tests[i] {
                Test::Accepts(j) => autos[j].accepts(state[j]),
                Test::Ends(node) => state[at_trie] == node,
            })
    };
    let mut cells = 0usize;

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
                    cells += next.len();
                    if states.len() >= STATE_BUDGET || cells > CELL_BUDGET {
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
        // One edge per successor, carrying every byte that leads there.
        out.sort_unstable_by_key(|&(j, _)| j);
        out.dedup_by(|(j, m), (k, n)| {
            let same = j == k;
            if same {
                *n += *m;
            }
            same
        });
        cells += out.len();
        if cells > CELL_BUDGET {
            return None;
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
                Auto::Trie(t) => {
                    if t.used[usize::from(b)] {
                        u16::from(b) + 1
                    } else {
                        0
                    }
                }
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
    /// The enumerated values: the trie node of the bytes so far; `u32::MAX` = no value
    /// has them as a prefix.
    Trie(Box<Trie>),
}

/// How an atom is decided on a product state.
#[derive(Clone, Copy)]
enum Test {
    /// The automaton at this index accepts.
    Accepts(usize),
    /// The trie is at this node.
    Ends(u32),
}

/// A trie of byte strings; node 0 is the root.
struct Trie {
    next: Vec<HashMap<u8, u32>>,
    used: [bool; 256],
}

impl Default for Trie {
    fn default() -> Trie {
        Trie {
            next: vec![HashMap::new()],
            used: [false; 256],
        }
    }
}

impl Trie {
    /// Adds `w`, returning its end node.
    fn insert(&mut self, w: &[u8]) -> u32 {
        let mut node = 0usize;
        for &b in w {
            self.used[usize::from(b)] = true;
            let fresh = self.next.len() as u32;
            let n = *self.next[node].entry(b).or_insert(fresh);
            if n == fresh {
                self.next.push(HashMap::new());
            }
            node = n as usize;
        }
        node as u32
    }
}

const TAGGED: u32 = 1 << 31;

/// An upper bound on the bytes one trie node takes (its map entry and its own map).
const TRIE_NODE_BYTES: usize = 64;

impl Auto {
    /// The bytes the automaton holds beyond its own size.
    fn bytes(&self) -> usize {
        match self {
            Auto::Dfa(d) => d.0.memory_usage(),
            _ => 0,
        }
    }

    fn start(&self) -> Option<u32> {
        match self {
            Auto::Dfa(d) => Some(d.start()),
            Auto::Len { cap, .. } => (*cap < u64::from(TAGGED)).then_some(0),
            Auto::Untagged | Auto::Trie(_) => Some(0),
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
            Auto::Trie(t) => t
                .next
                .get(s as usize)
                .and_then(|m| m.get(&b))
                .copied()
                .unwrap_or(u32::MAX),
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
            Auto::Trie(_) => false,
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
    /// Compiled automata (and failed builds), so each pattern and datatype is built once
    /// per thread, with the bytes they hold.
    static DFAS: RefCell<(HashMap<Key, Built>, usize)> = RefCell::new((HashMap::new(), 0));
}

/// A compiled automaton, or the failure to build one.
type Built = Result<Rc<Dfa>, ()>;

fn cached(key: Key, build: impl FnOnce() -> Result<Hir, ()>) -> Result<Rc<Dfa>, ()> {
    if let Some(d) = DFAS.with(|m| m.borrow().0.get(&key).cloned()) {
        return d;
    }
    let d = build().and_then(Dfa::build).map(Rc::new);
    let key_bytes = match &key {
        Key::Atom(Atom::Pattern(p) | Atom::Lang(p)) => p.len(),
        _ => 0,
    };
    let bytes = key_bytes + d.as_ref().map_or(0, |d| d.0.memory_usage());
    DFAS.with(|m| {
        let (m, total) = &mut *m.borrow_mut();
        if bytes > TOTAL_DFA_BYTES / 4 {
            return;
        }
        if m.len() >= 256 || *total + bytes > TOTAL_DFA_BYTES {
            m.clear();
            *total = 0;
        }
        *total += bytes;
        m.insert(key, d.clone());
    });
    d
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

/// Strings of XML characters (XML 1.0 `Char`), the strings of OWL 2 (§2.3).
fn xml_text() -> Hir {
    rep(
        class(&[
            ('\t', '\n'),
            ('\r', '\r'),
            ('\u{20}', '\u{D7FF}'),
            ('\u{E000}', '\u{FFFD}'),
            ('\u{10000}', '\u{10FFFF}'),
        ]),
        0,
        None,
    )
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

/// A lowercase BCP 47 `langtag` (RFC 5646 §2.1), the tags of `rdf:PlainLiteral` values
/// (its §2); private-use and grandfathered tags are not `langtag`s.
fn lang_tag() -> Hir {
    const PRIVATE: &str = "x(-[a-z0-9]{1,8})+";
    let langtag = [
        "([a-z]{2,3}(-[a-z]{3}){0,3}|[a-z]{4}|[a-z]{5,8})",
        "(-[a-z]{4})?",
        "(-([a-z]{2}|[0-9]{3}))?",
        "(-([a-z0-9]{5,8}|[0-9][a-z0-9]{3}))*",
        "(-[0-9a-wy-z](-[a-z0-9]{2,8})+)*",
    ]
    .concat()
        + &format!("(-{PRIVATE})?");
    regex_syntax::parse(&langtag).expect("a fixed regular expression")
}

/// The tags a basic language range matches (RFC 4647 §3.3.1): `*` matches every tag;
/// otherwise the tag equals the range or extends it by further subtags.
fn lang_range(range: &str) -> Hir {
    if range == "*" {
        return rep(class(&[('-', '-'), ('0', '9'), ('a', 'z')]), 1, None);
    }
    Hir::concat(vec![
        Hir::literal(range.as_bytes().to_vec()),
        rep(dash_then(subtag(1, false)), 0, None),
    ])
}

fn universe(space: Space) -> Result<Rc<Dfa>, ()> {
    cached(Key::Universe(space), || {
        Ok(match space {
            Space::Text => Hir::concat(vec![
                xml_text(),
                rep(Hir::concat(vec![sep(), lang_tag()]), 0, Some(1)),
            ]),
            Space::Uri | Space::Xml => xml_text(),
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
        // XML characters: three controls and three ranges.
        let scalars = 3 + (0xD800 - 0x20) + (0xFFFE - 0xE000) + 0x10_0000;
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
    fn basic_ranges_match_a_tag_prefix_at_subtag_boundaries() {
        let m = |r: &str, tag: &str| {
            holds(&F::atom(Atom::Lang(r.into())), &encode("x", Some(tag))).unwrap()
        };
        assert!(m("de-de", "de-de") && m("de-de", "de-de-1996") && m("de", "de-latn-de"));
        assert!(!m("de-de", "de-latn-de"), "basic filtering skips nothing");
        assert!(m("*", "fr") && !m("de", "deu") && !m("de-de", "de"));
        assert!(!holds(&F::atom(Atom::Lang("*".into())), &encode("x", None)).unwrap());
        assert!(is_language_range("de-DE") && is_language_range("*") && is_language_range("i"));
        for bad in ["de-*-DE", "*-de", "de--de", "", "d1", "de-123456789"] {
            assert!(!is_language_range(bad), "{bad}");
        }
    }

    #[test]
    fn tags_are_well_formed_bcp47() {
        let tagged = |tag: &str| {
            let w = encode("x", Some(tag));
            count(&F::atom(Atom::Is(w)), Space::Text, 1)
        };
        for ok in [
            "en",
            "en-gb",
            "zh-hant-tw",
            "de-ch-1901",
            "sl-rozaj-biske",
            "en-a-bbb-x-ccc",
            "zh-min-nan",
            "es-419",
            "zh-yue-hk",
        ] {
            assert_eq!(tagged(ok), Some(1), "{ok}");
        }
        for bad in [
            "en-a",
            "e",
            "en-gb-",
            "toolongtag",
            "en-x",
            "a-bbb",
            "en-12",
            "x-whatever",
            "i-klingon",
        ] {
            assert_eq!(tagged(bad), Some(0), "{bad}");
        }
        // `*` matches every well-formed tag and nothing else.
        let star = F::atom(Atom::Lang("*".into()));
        assert_eq!(
            count(&star.and(F::atom(Atom::Len(1, Some(1)))), Space::Text, 2),
            Some(u128::MAX)
        );
    }

    #[test]
    fn long_enumerations_keep_their_bytes_apart() {
        // `Q` and `R`, which no other automaton tells apart, first occur 65536 bytes apart.
        let mut w = b"Q".to_vec();
        w.extend(std::iter::repeat_n(b'a', 65535));
        w.push(b'R');
        assert_eq!(count(&F::atom(Atom::Is(w)), Space::Text, 2), Some(1));
    }

    #[test]
    fn enumerations_are_charged_before_the_walk() {
        // The walk would stop at the first byte (not an XML character), so only the
        // construction budget can give up here.
        let mut w = vec![0u8];
        w.extend(std::iter::repeat_n(b'a', TOTAL_DFA_BYTES / TRIE_NODE_BYTES));
        assert_eq!(count(&F::atom(Atom::Is(w)), Space::Text, 2), None);
        assert_eq!(
            count(&F::atom(Atom::Is(vec![0, b'a'])), Space::Text, 2),
            Some(0)
        );
    }

    #[test]
    fn oversized_formulas_give_up() {
        let many = |n: usize| {
            (0..n).fold(F::Const(false), |f, i| {
                f.or(F::atom(Atom::Is(format!("{i}").into_bytes())))
            })
        };
        assert_eq!(count(&many(1000), Space::Text, 2000), Some(1000));
        assert_eq!(count(&many(MAX_FORMULA_NODES), Space::Text, 2), None);
    }

    #[test]
    fn length_bounds_at_the_integer_limit_give_up() {
        assert_eq!(
            count(&F::atom(Atom::Len(0, Some(u64::MAX))), Space::Text, 2),
            None
        );
        assert_eq!(
            count(&F::atom(Atom::Len(u64::MAX, None)), Space::Text, 2),
            None
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
