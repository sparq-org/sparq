//! Notation3 (N3) rule reasoning — a forward-chaining engine with EYE/cwm parity
//! across the W3C N3 community-group test suite (reasoner manifest: 98.8% of run).
//!
//! N3 adds rules (`{ premise } => { conclusion }`), variables (`?x` and
//! `@forAll`/`@forSome` quantifiers), first-class lists (`( … )` is a TERM, not
//! rdf:first/rest structure), quoted formulae (`{ … }`; the empty formula `{}`
//! IS the literal `true`) and **builtins** (`math:`, `string:`, `log:`, …) on
//! top of Turtle. RDF-star / RDF 1.2 **quoted-triple terms** (`<< s p o >>` /
//! `<<( s p o )>>`, GH #2012) are first-class [`Term::Triple`] values: rule
//! premises match them structurally (variables inside the quotation bind,
//! nesting included), rule heads derive them, and ground quoted triples intern
//! into the dictionary through the content-addressed RDF 1.2 triple-term path. The engine: a parser (`parser`, with a STRICT Turtle mode),
//! a term model (`model`), and a semi-naive forward chainer that applies rules
//! to a fixpoint. Premises are reordered so each builtin runs only after the
//! atoms that can produce its inputs (cwm evaluates builtins "when ready").
//!
//! Builtins implemented (validated against the suite and EYE's own cases):
//!   * `math:` comparisons (greaterThan/lessThan/notGreaterThan/notLessThan/
//!     equalTo/notEqualTo, IEEE INF/NaN included) and functional arithmetic
//!     (sum/difference/product/quotient/max/min/exponentiation/negation/
//!     absoluteValue/rounded/floor/ceiling/remainder/integerQuotient/
//!     memberCount) — EXACT over integers/decimals (scaled i128, incl.
//!     decimal^int exponentiation), IEEE with NaN/INF propagation when any
//!     input is a double; `math:remainder` is integer-only with divisor-sign
//!     semantics (cwm). The real-valued family (sin..atanh, degrees/radians,
//!     logarithm, atan2 — `atan(x/y)` exactly like eye.pl) is always
//!     xsd:double, with REVERSE modes (`?y math:sin 0` solves y = asin 0).
//!   * `string:` concatenation (typed literals coerce to canonical value
//!     strings, IRIs to their text, like cwm)/length/containsIgnoringCase/
//!     containsRoughly/startsWith/endsWith/greaterThan/lessThan/notGreaterThan/
//!     notLessThan/equalIgnoringCase/notEqualIgnoringCase/matches/notMatches/
//!     replace/lowerCase/upperCase/scrape/format (%s %d %f %% subset — other
//!     directives fail the premise rather than mis-format), `encodeForUri`
//!     (XPath fn:encode-for-uri, see [`encode_for_uri`]) plus cwm's
//!     `encodeForURI`/`encodeForFragID` quoting pairs;
//!   * `list:` length/first/last/append, the member/in/iterate generators, and
//!     virtual `rdf:first`/`rdf:rest` access over list terms;
//!   * `time:` year/month/day/hour(s)/minute(s)/second(s)/dayOfWeek/timeZone
//!     and bidirectional inSeconds (epoch, deterministic — no wall-clock);
//!   * `log:` equalTo/notEqualTo, includes/notIncludes/supports (see below),
//!     conjunction (true = empty formula), conclusion (a formula's own
//!     closure), parsedAsN3, langlit, uri and dtlit (both directions),
//!     collectAllIn (scoped aggregation — findall over a clause's solutions)
//!     and forAllIn (scoped universal quantification), and — ONLY when the
//!     caller supplies a [`Resolver`] — semantics/content.
//!
//! `log:includes` containment is cwm-faithful (`formula_containment`): the
//! scope is the subject formula (the empty formula includes nothing); pattern
//! existentials (blanks, `@forSome`) are wildcards, pattern rule-variables
//! bind, scope-side quantified terms are opaque constants (the
//! quantifiers_limited matrix); virtual list access works inside containment;
//! an UNBOUND/non-formula subject falls back to store-scoped negation as
//! failure (this engine's documented idiom). `log:supports` first closes the
//! scope under its own `=>` rules.
//!
//! Backward rules (`<=`, log:isImpliedBy) are GOAL-DIRECTED, matching EYE:
//! they never fire forward; a forward-rule premise atom resolves against
//! backward conclusions SLD-style (standardized apart, depth-bounded by
//! the backward depth limit in `bounded`), with structural unification through lists and quoted
//! formulae on both sides.
//!
//! Conclusion blank nodes are EXISTENTIALS instantiated fresh once per
//! (rule, conclusion-binding) firing — cwm's quant-implies semantics.
//!
//! POLICY — document access: the engine performs NO I/O of its own; reasoning
//! is a pure function of its inputs. `log:semantics`/`log:content` evaluate
//! only when the caller passes a [`Resolver`]
//! ([`reason_n3_terms_with_resolver`]) deciding what an IRI may dereference
//! to (the conformance harness maps the suite's canonical IRIs into its
//! pinned local clone — strictly offline). `time:localTime`/`gmTime` stay
//! out (wall-clock reads), as does `log:rawType` (needs bnode-origin
//! tracking to match EYE) and `math:greaterThanOrEqual`-style names that are
//! not in the SWAP/EYE vocabulary.
//!
//! POLICY — import cycles: N3 is Turing-complete, so a `log:semantics` document
//! whose closure re-imports a document active up the resolution stack — directly
//! (A→A), indirectly (A→B→A), or via a re-used node in a diamond — would drive
//! the closure recursion forever under a LIVE resolver. The engine tracks the
//! formulae whose closure is in progress and, on re-entering one already in
//! progress, returns it UNCLOSED rather than recursing — cwm's "a document
//! already being loaded is not re-loaded" behaviour — so reasoning always
//! TERMINATES. A diamond that re-uses a shared document across SIBLING branches
//! is NOT a cycle and still resolves on every branch.
//!
//! Next increments: full `log:conclusion` parity on deep multi-document
//! closures (cwm_includes conclusion.n3 is the one remaining honest
//! reasoner-suite fail).

// [FABLE-5] sq-zgbso.3 (epic sq-zgbso, issue #1582): OPT-IN id-level compiled-rule
// evaluation for the scoped access-control N3 subset — see the module's own docs for the
// honest builtin/feature envelope. When the `compiled-rules` feature is off, zero of it
// is compiled (this hook is the module's only footprint in the default build).
#[cfg(feature = "compiled-rules")]
pub mod compiled;
mod model;
pub mod parser;
// [OPUS-5] sq-xqchl.2 (GH #3143) — the crate's single N3 writer (terms, statements, and
// whole rules). Rule serialization is what lets a caller emit EYE's "closure PLUS rules"
// output ([`reason_n3_pass_all`]) even though the chainer itself consumes rules.
pub mod serialize;
// Automatic stratification for store-scoped negation-as-failure (GH #6201, #5756).
pub(crate) mod bounded;
mod strata;
use bounded::{settle, Bounded, Truncation};
pub use bounded::ClosureLimits;
pub use strata::NegationCycles;

pub use model::{Rule, Term};
pub use serialize::{RuleKind, RuleVars};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};
use std::collections::HashMap;

/// One forward-chaining derivation step: `(derived ground fact, rule index, premise facts
/// that justified it)`. Collected during closure so a proof can be reconstructed.
type DerivationStep = ([Term; 3], usize, Vec<[Term; 3]>);

/// Facts + access indexes, so a rule-body join atom is an O(1)/O(matches) lookup instead of a
/// full scan. Indexed by (predicate, subject)→objects, (predicate, object)→subjects, and
/// predicate→facts — the patterns that arise when a rule atom's predicate (and often one
/// argument) is already bound. Maintained incrementally as the closure grows. Without this,
/// even semi-naive evaluation degrades to O(N²) on recursive rule chains (DeepTaxonomy).
#[derive(Default)]
struct FactIndex {
    all: FxHashSet<[Term; 3]>,
    ps: FxHashMap<(Term, Term), Vec<Term>>, // (pred, subj) -> objects
    po: FxHashMap<(Term, Term), Vec<Term>>, // (pred, obj) -> subjects
    p: FxHashMap<Term, Vec<[Term; 3]>>,     // pred -> facts (predicate-only-bound)
}

impl FactIndex {
    fn from_iter(facts: impl IntoIterator<Item = [Term; 3]>) -> FactIndex {
        let mut ix = FactIndex::default();
        for f in facts {
            ix.insert(f);
        }
        ix
    }
    fn contains(&self, t: &[Term; 3]) -> bool {
        self.all.contains(t)
    }
    fn insert(&mut self, t: [Term; 3]) -> bool {
        if !self.all.insert(t.clone()) {
            return false;
        }
        let [s, p, o] = &t;
        self.ps.entry((p.clone(), s.clone())).or_default().push(o.clone());
        self.po.entry((p.clone(), o.clone())).or_default().push(s.clone());
        self.p.entry(p.clone()).or_default().push(t.clone());
        true
    }
    /// Candidate facts matching a (partially-ground) pattern, via the most selective index.
    fn candidates(&self, s: &Term, p: &Term, o: &Term) -> Vec<[Term; 3]> {
        let (sg, pg, og) = (s.is_ground(), p.is_ground(), o.is_ground());
        if pg && sg {
            self.ps
                .get(&(p.clone(), s.clone()))
                .map(|os| os.iter().map(|ob| [s.clone(), p.clone(), ob.clone()]).collect())
                .unwrap_or_default()
        } else if pg && og {
            self.po
                .get(&(p.clone(), o.clone()))
                .map(|ss| ss.iter().map(|sb| [sb.clone(), p.clone(), o.clone()]).collect())
                .unwrap_or_default()
        } else if pg {
            self.p.get(p).cloned().unwrap_or_default()
        } else {
            self.all.iter().cloned().collect() // predicate unbound — rare; fall back to scan
        }
    }
}

const MATH: &str = "http://www.w3.org/2000/10/swap/math#";
const LOG: &str = "http://www.w3.org/2000/10/swap/log#";
const STRING: &str = "http://www.w3.org/2000/10/swap/string#";
const LIST: &str = "http://www.w3.org/2000/10/swap/list#";
const TIME: &str = "http://www.w3.org/2000/10/swap/time#";

/// An OPT-IN document accessor for `log:semantics` / `log:content`: maps an
/// IRI to that document's source text. The engine itself never touches the
/// filesystem or network — reasoning stays a pure function of its inputs
/// unless the caller supplies one of these.
pub type Resolver = dyn Fn(&str) -> Option<String>;

/// Keys of the quoted formulae whose closure is currently in progress up the call stack —
/// the import-cycle guard. `log:semantics` parses a resolved document into a quoted formula
/// which `log:supports` / `log:conclusion` then close (`formula_closure` → a NESTED
/// [`run_closure`]). Since N3 is Turing-complete, a document that imports itself (A→A),
/// imports a document that imports it (A→B→A), or is re-used across a diamond can otherwise
/// drive that `formula_closure` recursion forever with a LIVE resolver. We mark a formula's
/// content key on entry to its closure and remove it on exit (depth-first), so re-entering
/// the SAME closure that is already in progress is detected and broken, while sibling re-use
/// across a diamond (the marks are already popped) still succeeds. Shared (`Rc`) so the SAME
/// set is threaded into every nested closure of one top-level run, not reset per level.
/// Keying on formula CONTENT (not the document IRI) means the guard needs no extra plumbing
/// through the lazy `log:semantics` → `log:supports` evaluation hand-off, and is exact for
/// the recursion that actually loops (a formula closing itself, transitively).
///
/// The same shared state carries the runtime-document table, so every nested closure
/// draws runtime-parse blank labels from ONE table ([`run_shared::RuntimeDocs`]).
type VisitedDocs = std::rc::Rc<std::cell::RefCell<run_shared::RunShared>>;

pub(crate) use run_shared::RuntimeDocs;


/// The state one top-level run shares with all of its nested closures, and the table of
/// documents parsed at run time, which can outlive a run (a `MaterializedN3Graph` keeps
/// one for its whole life). Private fields and one constructor each: a run cannot hold a
/// table that was not made fresh against its sources.
mod run_shared {
    use super::{
        bounded, document_blank_labels, formula_bytes, fresh_blank_prefix, parser, Term,
        VisitedDocs,
    };
    use rustc_hash::{FxHashMap, FxHashSet};

    pub(super) struct RunShared {
        /// Content keys of the formulas whose closure is in progress (the import-cycle
        /// guard, [`VisitedDocs`]).
        pub(super) docs: FxHashSet<u64>,
        /// Blank labels for documents parsed at run time.
        runtime: RuntimeDocs,
    }

    /// The table of documents parsed at run time (`log:parsedAsN3`, `log:semantics`) and
    /// the ONE allocator of their blank labels. Such a document's blanks are its own: they
    /// are standardized apart into `{prefix}{key}_{label}`, `key` the document's
    /// scope key ([`runtime_doc_digest`], a specified hash of its text and base). So a label
    /// depends on the document alone, never on the order a run meets documents in, nor
    /// on the build: the same text under the same base always parses to the same formula
    /// (a materialization and a proof re-derivation agree, and exported proof keys are
    /// stable). Two documents never share a blank: a second document whose key is
    /// already taken is refused, failing the run. No runtime text can spell a label of
    /// the sources the table was made for: `prefix` is the smallest `__rp{k}_` no blank of
    /// any of them starts with, except a source blank spelled exactly as a runtime blank
    /// (`__rp{k}_{key}_<label>`), which names that runtime document's blank whether an
    /// exported closure or a person wrote it; so re-reasoning exported output keeps `k`.
    /// Per document the table keeps a constant-size entry (the full SHA-256 digest, for
    /// the collision refusal, and the label prefix), never the text or base, and it takes
    /// at most [`bounded::runtime_docs`] documents: one more fails the run with a cut. It
    /// finds a known document again by that digest (one SHA-256 per runtime parse).
    /// Cloning shares the table.
    #[derive(Clone)]
    pub(crate) struct RuntimeDocs(std::rc::Rc<std::cell::RefCell<DocTable>>);

    struct DocTable {
        prefix: String,
        /// Each known document's entry, by its full SHA-256 digest.
        by_doc: FxHashMap<[u8; 32], u32>,
        /// Each entry by its 8-byte scope key, for the collision refusal.
        by_key: FxHashMap<u64, u32>,
        /// One constant-size entry per document: its SHA-256 digest and label prefix.
        entries: Vec<DocEntry>,
        /// Documents this table may hold ([`bounded::runtime_docs`]).
        room: usize,
    }

    /// What the table keeps of one document: its full SHA-256 digest (to tell two
    /// documents sharing an 8-byte scope key apart) and its label prefix
    /// `{prefix}{key}_`, whose length is the same for every entry of a table.
    struct DocEntry {
        digest: [u8; 32],
        prefix: String,
    }

    /// Why the table refused a document.
    pub(super) enum Refusal {
        /// Another document has its scope key.
        Collision,
        /// The table is full ([`bounded::runtime_docs`]).
        Full,
    }

    impl RuntimeDocs {
        /// A table for runs over `sources` (every document whose blanks the runs hold: a
        /// query passes its data and its query document, a `MaterializedN3Graph` its
        /// rules document).
        pub(crate) fn for_sources(sources: &[&parser::Parsed]) -> RuntimeDocs {
            let mut labels = FxHashSet::default();
            for p in sources {
                labels.extend(document_blank_labels(p));
            }
            RuntimeDocs::for_labels(&labels)
        }

        /// [`for_sources`](Self::for_sources) given the sources' blank labels
        /// ([`document_blank_labels`]), for a run that has already collected them.
        pub(super) fn for_labels(labels: &FxHashSet<&str>) -> RuntimeDocs {
            // A source label spelled exactly as a runtime blank (`__rp{k}_{key}_<label>`)
            // is one: it was exported from an earlier run (`reason_n3_pass_all` output,
            // a serialized closure) and names that runtime document's blank. It does not
            // push `k` up, so re-reasoning an exported closure keeps the same `k` and the
            // reparsed document's blanks are the exported nodes (a fixed point).
            let other: FxHashSet<&str> =
                labels.iter().copied().filter(|l| !is_runtime_label(l)).collect();
            let prefix = fresh_blank_prefix(&other, "__rp");
            RuntimeDocs::with_room(prefix, bounded::runtime_docs())
        }

        fn with_room(prefix: String, room: usize) -> RuntimeDocs {
            let table = DocTable {
                prefix,
                by_doc: FxHashMap::default(),
                by_key: FxHashMap::default(),
                entries: Vec::new(),
                room,
            };
            RuntimeDocs(std::rc::Rc::new(std::cell::RefCell::new(table)))
        }

        /// The statements of the document parsed from `text` under `base` (by `parse`),
        /// with its blanks standardized apart; `Ok(None)` when `parse` fails or `admit`
        /// refuses the standardized formula's size, `Err` when the table refuses the
        /// document. `admit` gets that size ([`formula_bytes`] of the statements once
        /// every blank label carries its prefix) computed from the parsed statements,
        /// before any label is rewritten and before the table takes the document.
        fn formula(
            &self,
            text: &str,
            base: &str,
            parse: impl FnOnce() -> Option<Vec<[Term; 3]>>,
            admit: impl FnOnce(usize) -> bool,
        ) -> Result<Option<Vec<[Term; 3]>>, Refusal> {
            let Some(mut ts) = parse() else { return Ok(None) };
            // Each blank label grows by the label prefix, the same length for every
            // document of the table.
            let blanks = ts.iter().map(blank_occurrences).fold(0, usize::saturating_add);
            let grown = blanks.saturating_mul(self.0.borrow().label_prefix_len());
            let bytes = formula_bytes(ts.iter()).saturating_add(grown);
            if !admit(bytes) {
                return Ok(None);
            }
            // A document with no blank has nothing to standardize apart (and needs no key).
            if blanks == 0 {
                return Ok(Some(ts));
            }
            let mut t = self.0.borrow_mut();
            let prefix = t.prefix_for(text, base)?;
            // In place: only the blank labels are rewritten, nothing else is copied.
            fn go(t: &mut Term, p: &str) {
                match t {
                    Term::Blank(l) => l.insert_str(0, p),
                    Term::List(ms) => ms.iter_mut().for_each(|m| go(m, p)),
                    Term::Formula(rs) => rs.iter_mut().flatten().for_each(|m| go(m, p)),
                    Term::Triple(tr) => tr.iter_mut().for_each(|m| go(m, p)),
                    _ => {}
                }
            }
            ts.iter_mut().flatten().for_each(|t| go(t, prefix));
            debug_assert_eq!(formula_bytes(ts.iter()), bytes, "the admitted size is the size");
            Ok(Some(ts))
        }
    }

    /// The blank-label occurrences in `t`, through nested lists, formulas and quoted
    /// triples (walked with an explicit stack, as [`bounded::term_bytes`] walks).
    fn blank_occurrences(t: &[Term; 3]) -> usize {
        let mut n = 0usize;
        let mut stack: Vec<&Term> = t.iter().collect();
        while let Some(t) = stack.pop() {
            match t {
                Term::Blank(_) => n += 1,
                Term::List(ms) => stack.extend(ms),
                Term::Formula(rs) => stack.extend(rs.iter().flatten()),
                Term::Triple(tr) => stack.extend(tr.iter()),
                _ => {}
            }
        }
        n
    }

    /// The hex digits of a scope key in a label prefix.
    const KEY_HEX: usize = 16;

    impl DocTable {
        /// The length of every label prefix `{prefix}{key}_` of this table.
        fn label_prefix_len(&self) -> usize {
            self.prefix.len() + KEY_HEX + 1
        }

        /// The label prefix for the blanks of the document parsed from `text` under
        /// `base`; `Err` when another document has its key or the table is full (the
        /// table is then unchanged).
        fn prefix_for(&mut self, text: &str, base: &str) -> Result<&str, Refusal> {
            let digest = runtime_doc_digest(text, base);
            if let Some(&at) = self.by_doc.get(&digest) {
                return Ok(&self.entries[at as usize].prefix);
            }
            let key = scope_key(&digest);
            let at = match self.by_key.get(&key) {
                Some(&at) if self.entries[at as usize].digest == digest => at,
                Some(_) => return Err(Refusal::Collision),
                None => {
                    if self.entries.len() >= self.room {
                        return Err(Refusal::Full);
                    }
                    let mut prefix = String::with_capacity(self.label_prefix_len());
                    prefix.push_str(&self.prefix);
                    sparq_core::hex::encode_into(&digest[..8], false, &mut prefix);
                    prefix.push('_');
                    debug_assert_eq!(prefix.len(), self.label_prefix_len());
                    // An index; `room` keeps the table far below u32::MAX entries.
                    let at = u32::try_from(self.entries.len()).map_err(|_| Refusal::Full)?;
                    self.entries.push(DocEntry { digest, prefix });
                    self.by_key.insert(key, at);
                    at
                }
            };
            self.by_doc.insert(digest, at);
            Ok(&self.entries[at as usize].prefix)
        }

        /// The bytes the table keeps per document: the same for every document,
        /// whatever its text and base.
        #[cfg(test)]
        fn bytes_per_entry(&self) -> Vec<usize> {
            let slot = std::mem::size_of::<([u8; 32], u32)>() + std::mem::size_of::<(u64, u32)>();
            let entry = std::mem::size_of::<DocEntry>() + slot;
            self.entries.iter().map(|e| entry + e.prefix.capacity()).collect()
        }
    }

    /// Whether `label` has the exact shape of a standardized runtime blank:
    /// `__rp{k}_{key}_<label>` with `k` decimal digits and `key` 16 lowercase hex digits
    /// ([`runtime_doc_digest`]).
    pub(super) fn is_runtime_label(label: &str) -> bool {
        let Some(rest) = label.strip_prefix("__rp") else { return false };
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let Some(rest) = rest[digits..].strip_prefix('_') else { return false };
        let key = rest.as_bytes().get(..16);
        digits > 0
            && key.is_some_and(|k| k.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            && rest.as_bytes().get(16) == Some(&b'_')
    }

    /// The 8-byte scope key of a document: the first 8 bytes of its digest.
    fn scope_key(digest: &[u8; 32]) -> u64 {
        #[cfg(test)]
        if table_tests::SAME_KEY.with(std::cell::Cell::get) {
            return 0;
        }
        u64::from_be_bytes(digest[..8].try_into().expect("8 bytes"))
    }

    /// The SHA-256 digest of a document parsed at run time: SHA-256 over
    /// `"sparq-n3-runtime-doc\0"`, then the UTF-8 `text` and the UTF-8 `base`, each
    /// preceded by its byte length as a big-endian `u64`. Its first 8 bytes, as 16
    /// lowercase hex digits, are the document's scope key in its blank labels. Specified
    /// (not a std hasher), so the key is the same on every build: it reaches exported
    /// labels, proof keys and provenance IRIs.
    fn runtime_doc_digest(text: &str, base: &str) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"sparq-n3-runtime-doc\0");
        for part in [text, base] {
            h.update((part.len() as u64).to_be_bytes());
            h.update(part.as_bytes());
        }
        h.finalize().into()
    }

    /// The scope key of a document parsed at run time ([`runtime_doc_digest`]).
    #[cfg(test)]
    pub(crate) fn runtime_doc_key(text: &str, base: &str) -> String {
        sparq_core::hex::encode(&runtime_doc_digest(text, base)[..8])
    }

    impl RunShared {
        /// The shared state of one top-level run drawing on `runtime`.
        pub(super) fn new(runtime: RuntimeDocs) -> VisitedDocs {
            let shared = RunShared { docs: FxHashSet::default(), runtime };
            std::rc::Rc::new(std::cell::RefCell::new(shared))
        }

        /// The shared state of a top-level run over `sources`, with a table of its own.
        pub(super) fn for_sources(sources: &[&parser::Parsed]) -> VisitedDocs {
            RunShared::new(RuntimeDocs::for_sources(sources))
        }

        /// [`RuntimeDocs::formula`] on the run's table.
        pub(super) fn parsed_formula(
            &self,
            text: &str,
            base: &str,
            parse: impl FnOnce() -> Option<Vec<[Term; 3]>>,
            admit: impl FnOnce(usize) -> bool,
        ) -> Result<Option<Vec<[Term; 3]>>, Refusal> {
            self.runtime.formula(text, base, parse, admit)
        }
    }

    #[cfg(test)]
    mod table_tests {
        use super::{Refusal, RuntimeDocs, Term};

        std::thread_local! {
            /// Gives every document the same scope key, forcing a key collision.
            pub(super) static SAME_KEY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }

        /// Two different documents sharing a scope key are told apart by their full
        /// digest: the second is refused, while the first, met again, keeps its prefix.
        #[test]
        fn a_scope_key_collision_is_refused_and_a_known_document_is_kept() {
            SAME_KEY.with(|c| c.set(true));
            let t = RuntimeDocs::with_room("__rp0_".into(), 10);
            let a = t.formula("a", "", doc, |_| true);
            let b = t.formula("b", "", doc, |_| true);
            let again = t.formula("a", "", doc, |_| true);
            SAME_KEY.with(|c| c.set(false));
            assert!(matches!(b, Err(Refusal::Collision)));
            assert_eq!(a.ok().flatten(), again.ok().flatten());
            assert_eq!(t.0.borrow().entries.len(), 1);
        }


        /// A document of `text` whose one blank is `_:x`.
        fn doc() -> Option<Vec<[Term; 3]>> {
            let iri = |s: &str| Term::Iri(s.to_string());
            Some(vec![[Term::Blank("x".into()), iri("http://ex/p"), iri("http://ex/o")]])
        }

        /// When full the table refuses a new document without taking it, and still
        /// serves the documents it holds.
        #[test]
        fn a_full_table_refuses_without_inserting() {
            let t = RuntimeDocs::with_room("__rp0_".into(), 2);
            let a = t.formula("a", "", doc, |_| true).ok().flatten().expect("a");
            assert!(t.formula("b", "", doc, |_| true).is_ok());
            assert!(matches!(t.formula("c", "", doc, |_| true), Err(Refusal::Full)));
            assert_eq!(t.0.borrow().entries.len(), 2);
            assert_eq!(t.formula("a", "", doc, |_| true).ok().flatten(), Some(a));
            assert!(matches!(t.formula("c", "", doc, |_| true), Err(Refusal::Full)));
        }

        /// `admit` gets the size the formula has once standardized apart, and a refusal
        /// leaves the table without the document (nothing was prefixed).
        #[test]
        fn admit_sees_the_standardized_size_before_the_table_takes_the_document() {
            let t = RuntimeDocs::with_room("__rp0_".into(), 10);
            let mut seen = 0;
            let refused = t.formula("a", "", doc, |n| {
                seen = n;
                false
            });
            assert!(matches!(refused, Ok(None)));
            assert!(t.0.borrow().entries.is_empty());
            let kept = t.formula("a", "", doc, |_| true).ok().flatten().expect("kept");
            assert_eq!(super::formula_bytes(kept.iter()), seen);
            assert_eq!(t.0.borrow().entries.len(), 1);
        }

        /// The table keeps the same number of bytes for a one-byte text as for a
        /// megabyte one (and for a long base).
        #[test]
        fn an_entry_does_not_grow_with_its_document() {
            let t = RuntimeDocs::with_room("__rp0_".into(), 10);
            let big = "y".repeat(1 << 20);
            for (text, base) in [("a", ""), (big.as_str(), ""), ("a", big.as_str())] {
                assert!(t.formula(text, base, doc, |_| true).is_ok());
            }
            let sizes = t.0.borrow().bytes_per_entry();
            assert_eq!(sizes.len(), 3);
            assert!(sizes.iter().all(|&n| n == sizes[0]), "{sizes:?}");
        }
    }
}

/// A stable structural key for a quoted formula, used by the import-cycle guard
/// ([`VisitedDocs`]) to recognise a closure that is already in progress up the stack.
fn formula_key(ts: &[[Term; 3]]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    ts.hash(&mut h);
    h.finish()
}

/// Goal-directed context: the document's backward (`<=`) rules plus a counter for renaming
/// rule variables apart (standardizing apart, so nested applications of the same rule do not
/// capture each other's bindings); also carries the document base, the optional [`Resolver`]
/// for the document-access builtins, and the import-cycle guard ([`VisitedDocs`]).
struct BwCtx<'a> {
    rules: &'a [Rule],
    rename: std::cell::Cell<usize>,
    base: String,
    resolver: Option<&'a Resolver>,
    visited: VisitedDocs,
    /// The run's [`NegationCycles`] policy, inherited by nested `log:conclusion` closures.
    cycles: NegationCycles,
    /// The first error a nested closure met, or the first refusal of [`negation_gate`]; it
    /// fails the enclosing run.
    nested_error: std::cell::RefCell<Option<String>>,
    /// Why a search of this run (or of a nested closure it reached) was cut short, if one
    /// was ([`Truncation`]).
    truncated: Truncation,
    /// The current round's join-work allowance ([`bounded::JoinWork`]): every binding row
    /// a premise of this run produces, and every rule firing, is charged to it.
    work: bounded::JoinWork,
}

impl BwCtx<'_> {
    /// The statements `parse` makes of the document parsed at run time from `text` (under
    /// `base`), with its blanks standardized apart through the run's table
    /// ([`run_shared::RunShared::parsed_formula`]). `None` when the text does not parse,
    /// when `admit` refuses the size the standardized formula will have (checked before
    /// any label is rewritten), or when the table refuses the document (the refusal
    /// fails the run).
    fn runtime_formula(
        &self,
        text: &str,
        base: &str,
        parse: impl FnOnce() -> Option<Vec<[Term; 3]>>,
        admit: impl FnOnce(usize) -> bool,
        pending: &bounded::Pending,
    ) -> Option<Vec<[Term; 3]>> {
        let visited = self.visited.clone();
        let made = visited.borrow().parsed_formula(text, base, parse, admit);
        let e = match made {
            Ok(ts) => return ts,
            Err(run_shared::Refusal::Collision) => "n3: two documents parsed at run time share a \
                 blank-node scope key; refused rather than let their blank nodes alias"
                .to_string(),
            Err(run_shared::Refusal::Full) => bounded::runtime_docs_exceeded(pending),
        };
        self.nested_error.borrow_mut().get_or_insert(e);
        None
    }

    /// Use a budgeted or fallible step's result, recording a cut on the run
    /// ([`bounded::settle`]).
    fn settle<T>(&self, b: Bounded<T>) -> T {
        settle(&self.truncated, b)
    }
}

/// The ONE gate every negation and aggregation evaluation of the text engine passes
/// (`log:notIncludes`, `log:collectAllIn`, `log:forAllIn`, any scope), called after its
/// inner search: the set it read is complete only if the run is stratified (checked
/// before any matching by [`strata::stratify`]) and no search was cut short
/// ([`Truncation`]). Otherwise, unless the caller opted in to
/// [`NegationCycles::SinglePass`], the evaluation is refused: the binding is dropped and
/// the run fails with the reason (or, when a limit refused the run, with that refusal,
/// which the run returns anyway). Truncation is sticky and only grows, so a cut search
/// that happened before the negation, or during its inner search, is always seen here;
/// a later cut cannot change a set a stratified negation already read.
fn negation_gate(bw: &BwCtx) -> bool {
    let Some(why) = bw.truncated.get() else {
        return true;
    };
    if bw.cycles == NegationCycles::SinglePass {
        return true;
    }
    // A run refused by its ClosureLimits fails with that refusal, which names the limit.
    if bw.truncated.refusal().is_some() {
        return false;
    }
    bw.nested_error.borrow_mut().get_or_insert_with(|| {
        format!(
            "n3: refused a negation or aggregation (log:notIncludes / log:collectAllIn / \
             log:forAllIn) over an incomplete set: {why}, so missing facts would read as \
             absent. Opt in to NegationCycles::SinglePass to accept the incomplete result."
        )
    });
    false
}

impl<'a> BwCtx<'a> {
    /// A context sharing `visited` with the run it belongs to.
    fn new(rules: &'a [Rule], truncated: Truncation, visited: VisitedDocs) -> BwCtx<'a> {
        BwCtx {
            rules,
            rename: std::cell::Cell::new(0),
            base: String::new(),
            resolver: None,
            visited,
            cycles: NegationCycles::Reject,
            nested_error: std::cell::RefCell::new(None),
            work: bounded::JoinWork::forward(&truncated),
            truncated,
        }
    }
}

/// One derivation step: a `conclusion` triple was produced by rule `rule` (its index in the
/// document's rule order) from the ground `premises` (the supporting facts under the binding;
/// premise atoms proven by backward rules are not themselves facts and are not listed).
///
/// `conclusion_key` / `premise_keys` are the same facts' N3 identity keys
/// ([`serialize::statement_keys`]), taken from the N3 TERMS before list expansion and
/// interning — the ids alone cannot recover them (`()` interns as `rdf:nil`, a list as a
/// fresh blank chain, and the dictionary normalizes literal fields). Proof trees built
/// from these steps (`explain::n3_proof_tree`) take their node keys from here, so they
/// agree with `MaterializedN3Graph::why` on every fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofStep {
    pub conclusion: [Id; 3],
    pub rule: usize,
    pub premises: Vec<[Id; 3]>,
    pub conclusion_key: [String; 3],
    pub premise_keys: Vec<[String; 3]>,
}

/// Parse N3 `src`, run the rule closure, and return the entailed GROUND triples interned into
/// `dict`. The rules/formulae/variables are consumed by reasoning; only ground facts remain.
///
/// Store-scoped negation-as-failure (`log:notIncludes`, `log:collectAllIn`, `log:forAllIn`
/// over the current store) is STRATIFIED automatically: a rule that negates or aggregates
/// over a predicate the same document derives runs only after every rule deriving it has
/// reached its fixpoint. Anything the analysis cannot pin to one predicate counts as
/// EVERY predicate: a variable conclusion or premise predicate, a clause supplied through
/// a variable, and a nested closure (`log:conclusion`, `log:supports`). List builtins
/// depend on the `rdf:first`/`rdf:rest` triples they walk. A document whose negation
/// sits on a dependency cycle (including one through such an unknown dependency) cannot
/// be stratified and is REJECTED with an error naming the cycle, and so is a nested
/// closure over such a formula; [`reason_n3_with_cycles`] and
/// [`reason_n3_terms_with_cycles`] opt in to failing closed or to the legacy single-pass
/// evaluation instead ([`NegationCycles`]).
/// Dependencies are tracked per predicate, and per class for `rdf:type` with a constant
/// class: negating `?i a :Flagged` from a rule that concludes `?i a :Clean` is two strata,
/// while an `rdf:type` atom with a variable class still overlaps every class.
///
/// A negation or aggregation also refuses to read a store that some search of the run
/// cut short (the backward proof depth limit, the formula containment budget, the data
/// list walk cap, a nested closure left unclosed by the import-cycle guard): the run
/// fails rather than read missing facts as absent. Only
/// [`NegationCycles::SinglePass`] accepts that incomplete result.
///
/// # Errors
///
/// Returns an error for `@forAll` in a rule's own premise or a scoped body pattern,
/// as well as parse failures, negation cycles and incomplete negation reads.
pub fn reason_n3(dict: &mut Dict, src: &str) -> Result<Vec<[Id; 3]>, String> {
    reason_n3_in(dict, src, NegationCycles::Reject, &Truncation::top_level())
}

/// As [`reason_n3`], choosing what happens to rules that negate through a dependency
/// cycle ([`NegationCycles`]) instead of rejecting the document. Opting in to
/// [`NegationCycles::SinglePass`] is for rule sets whose caller already guarantees the
/// negated predicates are complete (for example by running the rules as their own
/// explicit stratum); the cycle diagnostic is available from
/// [`reason_n3_terms_with_cycles`].
pub fn reason_n3_with_cycles(
    dict: &mut Dict,
    src: &str,
    cycles: NegationCycles,
) -> Result<Vec<[Id; 3]>, String> {
    reason_n3_in(dict, src, cycles, &Truncation::top_level())
}

/// As [`reason_n3_with_cycles`], under the caller's forward-closure allowance
/// ([`ClosureLimits`]) instead of [`ClosureLimits::default`]. A closure that spends it
/// fails with an error naming the limit.
pub fn reason_n3_with_limits(
    dict: &mut Dict,
    src: &str,
    cycles: NegationCycles,
    limits: ClosureLimits,
) -> Result<Vec<[Id; 3]>, String> {
    reason_n3_in(dict, src, cycles, &Truncation::top_level_with(limits))
}

/// [`reason_n3_with_cycles`] recording into the caller's run record `truncated`.
fn reason_n3_in(
    dict: &mut Dict,
    src: &str,
    cycles: NegationCycles,
    truncated: &Truncation,
) -> Result<Vec<[Id; 3]>, String> {
    // No derivation tracking ([`StepMode::None`]): skips per-firing premise materialization
    // in the hot loop and the proof-step interning pass entirely.
    let parsed = bounded::parse_n3(src, "", truncated)?;
    let (facts, steps, _) =
        run_closure(parsed, None, None, truncated, StepMode::None, cycles, None)?.top_level();
    Ok(intern_closure::<NoKeys>(dict, &facts, &steps)?.closure)
}

/// As [`reason_n3`], but also return the derivation (a [`ProofStep`] for each NEWLY-derived
/// triple, in derivation order) — the EYE `--proof` analogue.
pub fn reason_n3_proof(dict: &mut Dict, src: &str) -> Result<(Vec<[Id; 3]>, Vec<ProofStep>), String> {
    let cuts = Truncation::top_level();
    let parsed = bounded::parse_n3(src, "", &cuts)?;
    let (facts, steps, _) = run_closure(
        parsed,
        None,
        None,
        &cuts,
        StepMode::Full,
        NegationCycles::Reject,
        None,
    )?
    .top_level();
    // The steps carry their facts' keys; the closure's own keys are not returned, so not built.
    let run = intern_closure::<NoKeys>(dict, &facts, &steps)?;
    Ok((run.closure, run.steps))
}

/// One [`reason_n3_proof_run`]: the closure as id triples, each closure fact's N3 identity
/// key, and the derivation steps — what `explain::n3_proof_tree` needs to tell apart
/// distinct N3 facts that intern to one id triple.
#[derive(Clone, Debug)]
pub struct N3ProofRun {
    /// The closure as interned triples (as [`reason_n3_proof`] returns it).
    pub closure: Vec<[Id; 3]>,
    /// Parallel to `closure`: each fact's identity key ([`serialize::statement_keys`] of
    /// the N3 statement it was interned from — asserted and derived facts alike; a list
    /// chain's rdf:first/rest rows are keyed as those rows).
    pub closure_keys: Vec<[String; 3]>,
    /// A [`ProofStep`] per newly-derived fact, in derivation order.
    pub steps: Vec<ProofStep>,
}

/// As [`reason_n3_proof`], keeping every closure fact's N3 identity key ([`N3ProofRun`]).
pub fn reason_n3_proof_run(dict: &mut Dict, src: &str) -> Result<N3ProofRun, String> {
    let cuts = Truncation::top_level();
    let parsed = bounded::parse_n3(src, "", &cuts)?;
    let (facts, steps, _) = run_closure(
        parsed,
        None,
        None,
        &cuts,
        StepMode::Full,
        NegationCycles::Reject,
        None,
    )?
    .top_level();
    intern_closure::<CollectKeys>(dict, &facts, &steps)
}

/// The EYE **`--pass-all`** / **`--pass-all-ground`** output document: the deductive
/// closure (base facts PLUS every entailed fact — what [`reason_n3`] computes) followed by
/// the document's own RULES, echoed back as N3 statements.
///
/// The chainer consumes rules, so `--pass`-style output alone loses them: feed a closure to
/// a second reasoner and it can derive nothing further. This entry point keeps the rules in
/// the document, which is exactly what the two `…_plus_rules` output modes of the eye-js
/// compatibility surface mean (`sq-xqchl.2`, GH #3143).
///
/// `vars` selects how rule variables are rendered — [`RuleVars::N3`] (`?x`, the
/// `--pass-all` form, re-parses as the same rule) or [`RuleVars::VarIris`] (SWAP `var:`
/// IRIs, the `--pass-all-ground` form, no syntactic variables left). Backward (`<=`) rules
/// are echoed too, with their arrow.
///
/// Output shape (**not** byte-compatible with EYE's own writer, which reconstructs
/// `@prefix` declarations and has its own statement order): full `<…>` IRIs, one statement
/// per line, closure statements SORTED (so the document is deterministic — the closure
/// itself is an unordered set), then rules in document order.
///
/// Re-running this over its own output is a FIXPOINT for a monotone rule set whose
/// conclusions mint no fresh existentials: the closure is already saturated and the rules
/// round-trip. A rule with a blank node in its CONCLUSION mints a fresh `_:__sk…` label per
/// firing, so such a document grows on each re-run — the same caveat EYE carries.
///
/// HONESTY: the mapping onto the two EYE flags is BY CONSTRUCTION — closure + echoed rules,
/// and "ground" read as "no syntactic variable survives IN A RULE" — not a differential
/// against an EYE binary (none runs in this repo's gates). Treat the document as sparq's own
/// `--pass-all` equivalent, not as byte-for-byte EYE output. [`RuleVars::VarIris`] grounds
/// the rules at every depth, quoted `{ … }` formulae included; it does NOT rewrite the
/// closure half, so a document that ASSERTS a formula-valued fact carrying a variable
/// (`:a :p { ?x :q :b }.` — data, not a rule) still echoes that `?x` verbatim.
///
/// # Errors
///
/// A parse error, or — the output must re-reason exactly as the input did — a closure fact
/// or rule that has no lossless N3 form ([`serialize::NotRepresentable`]): a unit mixing a
/// plain mention of an `@forAll` IRI with its universal (the one document-level declaration
/// would capture it), units whose plain mentions and universals need each other first (a
/// cycle), or a backward-chaining copy of a universal. No fallback spelling is written.
pub fn reason_n3_pass_all(src: &str, vars: RuleVars) -> Result<String, String> {
    let cuts = Truncation::top_level();
    let parsed = bounded::parse_n3(src, "", &cuts)?;
    // Clone the rules BEFORE the closure runs: `run_closure` reorders each premise for
    // builtin readiness, and the echo should reflect the document, not that plan.
    let (rules, backward_rules) = (parsed.rules.clone(), parsed.backward_rules.clone());
    let (facts, _steps, _) = run_closure(
        parsed,
        None,
        None,
        &cuts,
        StepMode::None,
        NegationCycles::Reject,
        None,
    )?
    .top_level();
    // Universals are written as their own IRIs under `@forAll` declarations, so the output
    // re-parses to the very same `__ua.<iri>` terms — the identity a second reasoning pass
    // produces too (GH #5391, GH #6701 review).
    let facts: Vec<&[Term; 3]> = facts.all.iter().collect();
    let echoed: Vec<(&Rule, RuleKind)> = rules
        .iter()
        .map(|r| (r, RuleKind::Forward))
        .chain(backward_rules.iter().map(|r| (r, RuleKind::Backward)))
        .collect();
    // Exact or refused: a closure fact or rule with no lossless N3 form (see
    // `serialize::Unit`) fails the call rather than writing a document that re-reasons
    // differently (GH #6701 review round 7).
    let mut out = String::new();
    serialize::write_document(&facts, &echoed, vars, &cuts, &mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// The EYE **`--query`** filter (`eye <data> --query <query.n3>`), at the TERM level: every
/// INSTANTIATED conclusion `Cθ` of the query document's `{ premise } => { conclusion }` rules,
/// for each binding `θ` satisfying the premise over the deductive closure of `data`.
///
/// This is a PROJECTION, not a closure step — a conclusion is emitted for every satisfying
/// binding INCLUDING one whose instantiated conclusion is already a fact of the closure (EYE
/// prints that answer; forward chaining would suppress it as "not new"). The query document's
/// own FACTS are not loaded as data (EYE reads the query file as a query, not as a second data
/// document); its backward (`<=`) rules ARE available to the premise, alongside the data
/// document's.
///
/// The premise is matched by the SAME matcher the forward chainer uses, so the FULL premise
/// language is available: builtins (`math:`/`string:`/`list:`/`log:`/`time:`, including the
/// scoped `log:collectAllIn`/`forAllIn` and negation-as-failure forms), quoted `{ … }`
/// formulae, first-class `( … )` lists and quoted `<< s p o >>` triples all evaluate exactly as
/// they do in a document rule (`sq-xqchl.1`, GH #3144 — the previous compat path translated the
/// premise to a SPARQL BGP, which cannot evaluate any of them and so rejected them).
///
/// Answers come back in rule-then-binding order, DEDUPLICATED. Blank nodes appearing only in a
/// query conclusion are existentials, instantiated fresh once per distinct conclusion-relevant
/// binding — the same quant-implies semantics [`reason_n3`] gives a document rule.
///
/// Store-scoped negation is stratified as in [`reason_n3`], over the rule set the call
/// evaluates: the data document's rules plus the query document's backward rules and
/// forward premises. A negation cycle in it, including one a query-supplied backward rule
/// closes, is an error.
///
/// Errors for unsupported antecedent quantifiers, when either document fails to parse,
/// or when the query document has no forward rule
/// (a fact-only or backward-only query document has nothing to project — fail loudly rather
/// than return an empty answer that reads like "the query matched nothing").
pub fn reason_n3_query_terms(data: &str, query: &str) -> Result<Vec<[Term; 3]>, String> {
    query_terms_in(data, query, Truncation::top_level())
}

/// [`reason_n3_query_terms`] recording into the caller's run record `cuts`.
fn query_terms_in(data: &str, query: &str, cuts: Truncation) -> Result<Vec<[Term; 3]>, String> {
    let data_parsed = bounded::parse_n3(data, "", &cuts)?;
    let query_parsed = bounded::parse_n3(query, "", &cuts)?;
    validate_rule_quantifiers(&query_parsed)?;
    if query_parsed.rules.is_empty() {
        return Err("n3 query filter: the query document contains no `{ … } => { … }` forward \
                    rule; only forward-rule (SELECT/CONSTRUCT-style) query documents project an \
                    answer"
            .to_string());
    }
    // Namespace for query-conclusion existentials: fresh against BOTH documents' blank labels,
    // and (by family) disjoint from the `__sk…` labels `run_closure` mints inside the closure.
    let sk_prefix = {
        let mut seen = document_blank_labels(&data_parsed);
        seen.extend(document_blank_labels(&query_parsed));
        fresh_blank_prefix(&seen, "__qsk")
    };
    // Backward rules reachable from the query premise: the data document's (which the closure
    // itself used) plus the query document's own. `run_closure` reorders backward premises for
    // builtin readiness before building its context, so do the same for this copy.
    let base = data_parsed.base.clone();
    let mut backward: Vec<Rule> =
        data_parsed.backward_rules.iter().chain(&query_parsed.backward_rules).cloned().collect();
    for r in &mut backward {
        r.premise = order_premise(&r.premise);
    }
    // Stratification check over the rule set this call actually evaluates, before any
    // matching: the data document's forward rules, the backward rules of BOTH documents,
    // and the query's forward rules as consumers. A query conclusion is projected, never
    // added to the store, so those rules produce nothing. A negation cycle anywhere in it
    // (e.g. a self-negating backward rule supplied by the query) is an error.
    let forward: Vec<Rule> = data_parsed
        .rules
        .iter()
        .cloned()
        .chain(query_parsed.rules.iter().map(|r| Rule {
            premise: r.premise.clone(),
            conclusion: Vec::new(),
        }))
        .collect();
    strata::stratify(&forward, &backward, NegationCycles::Reject)?;
    // One run: the data closure and the query premises share one cut record.
    let truncated = cuts;
    // ... and one runtime-parse allocator, fresh against both documents: a text the query
    // parses never spells a blank of the data closure.
    let shared = run_shared::RunShared::for_sources(&[&data_parsed, &query_parsed]);
    let facts = run_closure(
        data_parsed,
        None,
        Some(shared.clone()),
        &truncated,
        StepMode::None,
        NegationCycles::Reject,
        None,
    )?
    .read_further(&truncated);
    let mut bw = BwCtx::new(&backward, truncated, shared);
    bw.base = base;

    let mut out: Vec<[Term; 3]> = Vec::new();
    let mut emitted: FxHashSet<[Term; 3]> = FxHashSet::default();
    let mut sk_counter = 0usize;
    for (ri, rule) in query_parsed.rules.iter().enumerate() {
        let premise = order_premise(&rule.premise);
        let (concl_blanks, concl_vars) = conclusion_existentials(&rule.conclusion);
        let mut fired: FxHashSet<String> = FxHashSet::default();
        for b in match_premise(&premise, &facts, &bw) {
            let sk: Option<FxHashMap<String, String>> = if concl_blanks.is_empty() {
                None
            } else {
                let key: String = concl_vars.iter().map(|v| format!("{:?};", b.get(v))).collect();
                if !fired.insert(key) {
                    continue; // this firing already instantiated its existentials
                }
                sk_counter += 1;
                Some(
                    concl_blanks
                        .iter()
                        .map(|l| (l.clone(), format!("{}{}_{}_{}", sk_prefix, ri, sk_counter, l)))
                        .collect(),
                )
            };
            for c in &rule.conclusion {
                let c = match &sk {
                    Some(map) => rename_blanks(c, map),
                    None => c.clone(),
                };
                if let Some(g) = ground_triple(&c, &b, &bw.truncated) {
                    // A conclusion past the term-depth limit is dropped, as a cut.
                    if bw.settle(bounded::generated(&bw.truncated, &g)) && emitted.insert(g.clone())
                    {
                        out.push(g);
                    }
                }
            }
        }
    }
    if let Some(e) = bw.nested_error.take().or_else(|| bw.truncated.refusal()) {
        return Err(e);
    }
    Ok(out)
}

/// As [`reason_n3_query_terms`], but interned into `dict` — the RDF-shaped answer form
/// [`reason_n3`] returns. First-class `( … )` list values in an answer are expanded into
/// rdf:first/rest blank-node chains (whose structure triples are part of the returned answer),
/// exactly as the closure entry points expand them. A generated chain cell is always a
/// DISTINCT node from any blank the answers already carry — cell labels are minted under a
/// prefix proven fresh against them, so a projected `_:_l1` cannot merge with a list cell.
///
/// Errors — additionally to [`reason_n3_query_terms`] — when an answer carries a quoted `{ … }`
/// formula, which has no dictionary representation; use the term-level entry point for a query
/// whose conclusion is formula-valued.
pub fn reason_n3_query(dict: &mut Dict, data: &str, query: &str) -> Result<Vec<[Id; 3]>, String> {
    let answers = query_terms_in(data, query, Truncation::top_level())?;
    let mut exp = ListExpander::new(&answers);
    let (mut rows, mut structure) = exp.expand_rows(&answers);
    rows.append(&mut structure);
    let mut out = Vec::with_capacity(rows.len());
    for t in &rows {
        out.push([intern(dict, &t[0])?, intern(dict, &t[1])?, intern(dict, &t[2])?]);
    }
    Ok(out)
}

/// A rule conclusion's EXISTENTIALS (its blank labels — instantiated fresh once per firing) and
/// its VARIABLES (the firing key: one instantiation per distinct conclusion-relevant binding,
/// so a rule re-evaluated against the same bindings does not mint endless new blanks).
///
/// Quoted triples are transparent structure — their blanks are conclusion existentials and
/// their variables part of the firing key — whereas a quoted `{ … }` formula is an opaque VALUE
/// whose inner terms are formula-scoped, so it is not descended into.
fn conclusion_existentials(conclusion: &[[Term; 3]]) -> (Vec<String>, Vec<String>) {
    fn scan(
        t: &Term,
        blanks: &mut FxHashSet<String>,
        vars: &mut std::collections::BTreeSet<String>,
    ) {
        match t {
            Term::Blank(l) => {
                blanks.insert(l.clone());
            }
            Term::Var(v) => {
                vars.insert(v.clone());
            }
            Term::List(ms) => ms.iter().for_each(|m| scan(m, blanks, vars)),
            Term::Triple(t) => t.iter().for_each(|m| scan(m, blanks, vars)),
            _ => {}
        }
    }
    let mut blanks = FxHashSet::default();
    let mut vars = std::collections::BTreeSet::new();
    for row in conclusion {
        for t in row {
            scan(t, &mut blanks, &mut vars);
        }
    }
    (blanks.into_iter().collect(), vars.into_iter().collect())
}

/// A stratified run's result ([`reason_n3_stratified`]).
pub struct StratifiedN3Closure {
    /// The FINAL stratum's ground closure, interned into the dictionary (the
    /// same form [`reason_n3`] returns).
    pub facts: Vec<[Id; 3]>,
    /// Term-level closure size after each stratum, in stratum order (counted
    /// before rdf:first/rest list expansion and interning) — the per-stratum
    /// stats hook a stratified pipeline records.
    pub strata_facts: Vec<usize>,
    /// Each document's stratification diagnostic under [`NegationCycles::FailClosed`] /
    /// [`NegationCycles::SinglePass`] ([`reason_n3_stratified_with_cycles`]), in stratum
    /// order. A FailClosed skip also makes every later document's negation or
    /// aggregation refuse the carried closure (the run errors).
    pub warnings: Vec<String>,
}

/// Run the rule closure STRATUM BY STRATUM: each `strata[i]` is a complete N3
/// document (facts + rules), and the full term-level closure of the strata
/// before it is carried forward IN MEMORY as additional input facts — no
/// serialize/re-parse round-trip between strata (formula-valued facts, which
/// a text round-trip cannot represent, carry over intact).
///
/// Each document is itself stratified automatically (see [`reason_n3`]) and refuses a
/// negation cycle of its own, even when an earlier stratum completes the negated
/// predicate (the per-document analysis cannot know that). Explicit strata are for
/// pipelines that ship their rules as separate documents; a stratum the analysis still
/// refuses needs [`reason_n3_stratified_with_cycles`] with
/// [`NegationCycles::SinglePass`], and then the caller guarantees completeness. The engine's NON-MONOTONIC premise operators (store-scoped
/// `log:notIncludes`, `log:collectAllIn` / `log:forAllIn`) are reliable over
/// predicates FULLY PRESENT before their stratum starts (derived facts are never
/// retracted), which an earlier stratum here guarantees.
///
/// Blank-node scope is PER STRATUM, exactly as if each stratum were its own
/// re-parsed document: carried blank nodes (input blanks and minted rule
/// existentials) are renamed apart at every stratum boundary — co-reference
/// within the carried closure is preserved, and a label reused in a later
/// stratum's source stays a DISTINCT node.
///
/// One stratum is exactly [`reason_n3`]; zero strata yield an empty closure.
pub fn reason_n3_stratified(
    dict: &mut Dict,
    strata: &[&str],
) -> Result<StratifiedN3Closure, String> {
    stratified_in(
        dict,
        strata,
        NegationCycles::Reject,
        Truncation::top_level(),
    )
}

/// As [`reason_n3_stratified`], choosing what happens to rules that negate through a
/// dependency cycle inside one stratum ([`NegationCycles`]). Under
/// [`NegationCycles::FailClosed`] the closure a stratum carries forward lacks what its
/// skipped rules would derive, so it is INCOMPLETE: every negation or aggregation in a
/// later stratum refuses it and the run errors (the skipped facts would read as absent).
/// The diagnostics are in [`StratifiedN3Closure::warnings`].
pub fn reason_n3_stratified_with_cycles(
    dict: &mut Dict,
    strata: &[&str],
    cycles: NegationCycles,
) -> Result<StratifiedN3Closure, String> {
    stratified_in(dict, strata, cycles, Truncation::top_level())
}

/// [`reason_n3_stratified_with_cycles`] recording into the caller's run record
/// `truncated`, which the whole pipeline shares.
fn stratified_in(
    dict: &mut Dict,
    strata: &[&str],
    cycles: NegationCycles,
    truncated: Truncation,
) -> Result<StratifiedN3Closure, String> {
    let mut carried: Vec<[Term; 3]> = Vec::new();
    let mut facts = FactIndex::default();
    let mut strata_facts = Vec::with_capacity(strata.len());
    let mut warnings = Vec::new();
    // One truncation flag for the whole pipeline: a cut search in an earlier stratum
    // leaves the facts it carries forward incomplete, so a later stratum's negation gate
    // must see it.
    for (i, src) in strata.iter().enumerate() {
        let mut parsed = bounded::parse_n3(src, "", &truncated)?;
        if !carried.is_empty() {
            // Rename carried blanks (input blanks and minted `__sk…` rule
            // existentials) apart from this stratum's own labels. The prefix
            // is chosen fresh against every blank label the stratum's source
            // actually uses, so even a literal `_:__st0_b` in the source
            // cannot capture a carried node, and `__st…` never collides with
            // the `__sk…` labels this stratum's closure mints (whose counter
            // restarts). One uniform injective rename preserves co-reference
            // within the carried closure.
            let prefix = fresh_carry_prefix(&parsed);
            for t in &mut carried {
                *t = stratum_blanks(t, &prefix);
            }
        }
        parsed.facts.append(&mut carried);
        // An incomplete closure (rules FailClosed skipped) taints the whole rest of the
        // pipeline: every later document's negation gate reads `truncated`.
        // Each stratum's runtime parses are standardized apart against its own source,
        // which holds the earlier strata's carried (renamed) closure.
        let shared = run_shared::RunShared::for_sources(&[&parsed]);
        let closure = run_closure(
            parsed,
            None,
            Some(shared),
            &truncated,
            StepMode::None,
            cycles,
            None,
        )?;
        warnings.extend(closure.warning.iter().map(|w| format!("stratum {i}: {w}")));
        let f = closure.read_further(&truncated);
        strata_facts.push(f.all.len());
        if i + 1 < strata.len() {
            carried = f.all.iter().cloned().collect();
        }
        facts = f;
    }
    Ok(StratifiedN3Closure {
        facts: intern_closure::<NoKeys>(dict, &facts, &[])?.closure,
        strata_facts,
        warnings,
    })
}

/// The smallest `__st{k}_` prefix that no blank label anywhere in `parsed`
/// (facts and rule premises/conclusions alike, recursing through lists,
/// formulae, and quoted triples) starts with. Renaming a carried label `l`
/// to `__st{k}_{l}` therefore yields a label proven absent from the
/// stratum's source — and one that can never collide with the `__sk…`
/// labels [`run_closure`] mints for rule existentials.
fn fresh_carry_prefix(parsed: &parser::Parsed) -> String {
    let seen = document_blank_labels(parsed);
    fresh_blank_prefix(&seen, "__st")
}

/// The namespace [`MaterializedN3Graph`](crate::MaterializedN3Graph) gives its caller's
/// blank labels (GH #6775): the smallest `__bs{k}_` prefix no blank label of the rules
/// document starts with. The parser's own labels (`_b<n>` for `[]`, `_:x` as written,
/// `__path<n>`, `__ex_<local>`) are all in `parsed`, so none starts with it, and the
/// `__sk…`/`__st…`/`__rp…` labels the engine mints are other families.
pub(crate) fn fresh_base_blank_prefix(parsed: &parser::Parsed) -> String {
    fresh_blank_prefix(&document_blank_labels(parsed), "__bs")
}

/// Every blank label reachable in `t`, including nested list members, formula
/// rows and quoted-triple components.
fn term_blank_labels<'a>(t: &'a Term, out: &mut FxHashSet<&'a str>) {
    match t {
        Term::Blank(l) => {
            out.insert(l.as_str());
        }
        Term::List(ms) => ms.iter().for_each(|m| term_blank_labels(m, out)),
        Term::Formula(ts) => {
            ts.iter().for_each(|r| r.iter().for_each(|m| term_blank_labels(m, out)));
        }
        Term::Triple(tr) => tr.iter().for_each(|m| term_blank_labels(m, out)),
        _ => {}
    }
}

/// Every blank label in a parsed document, including nested structured terms.
fn document_blank_labels(parsed: &parser::Parsed) -> FxHashSet<&str> {
    let mut seen = FxHashSet::default();
    let rule_terms = parsed
        .rules
        .iter()
        .chain(&parsed.backward_rules)
        .flat_map(|r| r.premise.iter().chain(&r.conclusion));
    for t in parsed.facts.iter().chain(rule_terms) {
        t.iter().for_each(|m| term_blank_labels(m, &mut seen));
    }
    seen
}

/// The smallest numbered `family{k}_` prefix that no existing label starts with.
fn fresh_blank_prefix(seen: &FxHashSet<&str>, family: &str) -> String {
    let mut taken = FxHashSet::default();
    for label in seen {
        let Some(suffix) = label.strip_prefix(family) else {
            continue;
        };
        let Some((digits, _)) = suffix.split_once('_') else {
            continue;
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // not-a-cut: not-evaluation (blank-label numbering; a label past usize is not ours)
        if let Ok(k) = digits.parse::<usize>() {
            taken.insert(k);
        }
    }

    let mut k = 0usize;
    while taken.contains(&k) {
        k += 1;
    }
    format!("{}{}_", family, k)
}

/// `t` with every blank label pushed into the carried namespace `prefix`
/// (recursing into lists, formulae, and quoted triples) — the per-stratum
/// document scope of [`reason_n3_stratified`]. `prefix` comes from
/// [`fresh_carry_prefix`]; prefixing is injective, so co-reference among
/// carried blanks is preserved.
fn stratum_blanks(t: &[Term; 3], prefix: &str) -> [Term; 3] {
    fn go(t: &Term, p: &str) -> Term {
        match t {
            Term::Blank(l) => Term::Blank(format!("{}{}", p, l)),
            Term::List(ms) => Term::List(ms.iter().map(|m| go(m, p)).collect()),
            Term::Formula(ts) => Term::Formula(
                ts.iter().map(|r| [go(&r[0], p), go(&r[1], p), go(&r[2], p)]).collect(),
            ),
            Term::Triple(tr) => {
                Term::Triple(Box::new([go(&tr[0], p), go(&tr[1], p), go(&tr[2], p)]))
            }
            _ => t.clone(),
        }
    }
    [go(&t[0], prefix), go(&t[1], prefix), go(&t[2], prefix)]
}

/// TERM-level closure + full derivation steps `(conclusion, rule index, premises)` —
/// the `explain` feature's N3 entry point ([`crate::MaterializedN3Graph::why`]).
#[cfg(feature = "explain")]
#[allow(clippy::type_complexity)]
pub(crate) fn reason_n3_terms_proof(
    src: &str,
    extra: impl IntoIterator<Item = [Term; 3]>,
    cuts: &Truncation,
    runtime: Option<&RuntimeDocs>,
) -> Result<(FxHashSet<[Term; 3]>, Vec<DerivationStep>), String> {
    let parsed = bounded::parse_n3_with_extra(src, "", extra, cuts)?;
    let (facts, steps, _) = run_closure(
        parsed,
        None,
        None,
        cuts,
        StepMode::Full,
        NegationCycles::Reject,
        runtime,
    )?
    .top_level();
    Ok((facts.all, steps))
}

/// Term-level closure result — what [`reason_n3_terms`] returns. Unlike
/// [`reason_n3`], nothing is interned: facts may contain `{ … }` formula terms
/// (which have no dictionary representation) and survive here untouched.
pub struct N3Closure {
    /// The full ground closure: original facts plus every derivation (a set).
    pub facts: Vec<[Term; 3]>,
    /// Only the newly-derived triples, in derivation order.
    pub derived: Vec<[Term; 3]>,
    /// Forward (`=>`) / backward (`<=`) rule counts of the parsed document.
    pub n_rules: usize,
    pub n_backward_rules: usize,
    /// Non-fatal diagnostics. Today: under [`NegationCycles::FailClosed`] /
    /// [`NegationCycles::SinglePass`] ([`reason_n3_terms_with_cycles`]), the negation
    /// cycle that made some rules unstratifiable. Empty for every stratifiable document
    /// (the default entry points reject an unstratifiable one).
    pub warnings: Vec<String>,
}

/// As [`reason_n3`], but returns the closure at the TERM level (no dictionary)
/// and resolves relative IRIs against `base` when given — the entry point used
/// by the W3C N3 conformance harness (cwm/EYE-style: `--think` then compare).
pub fn reason_n3_terms(src: &str, base: Option<&str>) -> Result<N3Closure, String> {
    reason_n3_terms_in(
        src,
        base,
        None,
        NegationCycles::Reject,
        &Truncation::top_level(),
    )
}

/// As [`reason_n3_terms`], with an optional document [`Resolver`] enabling the
/// `log:semantics` / `log:content` builtins (policy: document access is OFF by
/// default and the engine performs no I/O of its own — the caller decides what
/// an IRI may dereference to, e.g. the conformance harness maps the suite's
/// canonical IRIs to its local clone).
pub fn reason_n3_terms_with_resolver(
    src: &str,
    base: Option<&str>,
    resolver: Option<&Resolver>,
) -> Result<N3Closure, String> {
    reason_n3_terms_in(
        src,
        base,
        resolver,
        NegationCycles::Reject,
        &Truncation::top_level(),
    )
}

/// As [`reason_n3_terms_with_resolver`], choosing what happens to rules that negate
/// through a dependency cycle ([`NegationCycles`]). Every other N3 entry point REJECTS
/// such a document; [`NegationCycles::FailClosed`] skips the affected rules and
/// [`NegationCycles::SinglePass`] (legacy, may fail open) runs them single-pass after the
/// stratifiable rules. Both report the cycle in [`N3Closure::warnings`].
pub fn reason_n3_terms_with_cycles(
    src: &str,
    base: Option<&str>,
    resolver: Option<&Resolver>,
    cycles: NegationCycles,
) -> Result<N3Closure, String> {
    reason_n3_terms_in(src, base, resolver, cycles, &Truncation::top_level())
}

/// As [`reason_n3_terms_with_cycles`], under the caller's forward-closure allowance
/// ([`ClosureLimits`]) instead of [`ClosureLimits::default`]. A closure that spends it
/// fails with an error naming the limit.
pub fn reason_n3_terms_with_limits(
    src: &str,
    base: Option<&str>,
    resolver: Option<&Resolver>,
    cycles: NegationCycles,
    limits: ClosureLimits,
) -> Result<N3Closure, String> {
    reason_n3_terms_in(
        src,
        base,
        resolver,
        cycles,
        &Truncation::top_level_with(limits),
    )
}

/// [`reason_n3_terms_with_cycles`] for a crate-internal caller that already has a run:
/// every cut is recorded on its record `truncated` (e.g. the incremental graph's
/// fallback).
pub(crate) fn reason_n3_terms_in(
    src: &str,
    base: Option<&str>,
    resolver: Option<&Resolver>,
    cycles: NegationCycles,
    truncated: &Truncation,
) -> Result<N3Closure, String> {
    let parsed = bounded::parse_n3(src, base.unwrap_or(""), truncated)?;
    let (n_rules, n_backward_rules) = (parsed.rules.len(), parsed.backward_rules.len());
    // `derived` needs the conclusions in derivation order but never the premises.
    let (facts, steps, warning) = run_closure(
        parsed,
        resolver,
        None,
        truncated,
        StepMode::Conclusions,
        cycles,
        None,
    )?
    .top_level();
    Ok(N3Closure {
        facts: facts.all.into_iter().collect(),
        derived: steps.into_iter().map(|(g, _, _)| g).collect(),
        n_rules,
        n_backward_rules,
        warnings: warning.into_iter().collect(),
    })
}

/// The term-level closure of the rules document `src` plus `extra` statements handed over
/// AS TERMS ([`bounded::parse_n3_with_extra`]) — the lossless re-reasoning entry point of the
/// incremental N3 fallback. `extra` is classified exactly as statements written at the end
/// of `src` would be (so `log:implies`-family triples still become rules), but no term is
/// serialized, so `@forAll` universals keep their identity.
pub(crate) fn reason_n3_terms_with_facts(
    src: &str,
    extra: impl IntoIterator<Item = [Term; 3]>,
    cycles: NegationCycles,
    truncated: &Truncation,
    runtime: &RuntimeDocs,
) -> Result<N3Closure, String> {
    let parsed = bounded::parse_n3_with_extra(src, "", extra, truncated)?;
    let (n_rules, n_backward_rules) = (parsed.rules.len(), parsed.backward_rules.len());
    let (facts, steps, warning) = run_closure(
        parsed,
        None,
        None,
        truncated,
        StepMode::Conclusions,
        cycles,
        Some(runtime),
    )?
    .top_level();
    Ok(N3Closure {
        facts: facts.all.into_iter().collect(),
        derived: steps.into_iter().map(|(g, _, _)| g).collect(),
        n_rules,
        n_backward_rules,
        warnings: warning.into_iter().collect(),
    })
}

/// How much derivation history [`run_closure`] records. Premise materialization (the
/// supporting ground facts of each firing) costs an extra instantiation + fact lookup per
/// premise atom per NEW fact, and the conclusions an extra clone — skip whatever the caller
/// will discard.
#[derive(Clone, Copy, PartialEq)]
enum StepMode {
    /// No steps at all (closure-only callers: [`reason_n3`], `formula_closure`).
    None,
    /// `(conclusion, rule, [])` per new fact — derivation order without premises
    /// ([`reason_n3_terms`]'s `derived`).
    Conclusions,
    /// Full [`ProofStep`] data ([`reason_n3_proof`]).
    Full,
}

/// One document's closure, as [`run_closure`] hands it to its caller.
struct Closure {
    /// The final fact set. It is a cut when [`NegationCycles::FailClosed`] skipped rules
    /// on a negation cycle: the closure then lacks whatever those rules derive, so a
    /// later reader that negates or aggregates over it would read missing facts as
    /// absent. Whoever reads the closure further (the next document of an explicit
    /// pipeline, a query, a nested closure's consumer) settles it on its own run's
    /// [`Truncation`], which every later [`negation_gate`] checks.
    facts: Bounded<FactIndex>,
    /// The derivation steps `(conclusion, rule index, supporting premises)` in derivation
    /// order (as much of them as the [`StepMode`] asks for).
    steps: Vec<DerivationStep>,
    /// The stratification diagnostic of a negation cycle evaluated under
    /// [`NegationCycles::FailClosed`] or [`NegationCycles::SinglePass`].
    warning: Option<String>,
}

impl Closure {
    /// The closure a TOP-LEVEL entry point returns to its own caller: nothing inside the
    /// engine reads it further, and the facts are exactly what [`NegationCycles::FailClosed`]
    /// promises (the skipped rules derive nothing; [`N3Closure::warnings`] names them).
    fn top_level(self) -> (FactIndex, Vec<DerivationStep>, Option<String>) {
        let returned = std::cell::Cell::new(false);
        (settle(&returned, self.facts), self.steps, self.warning)
    }

    /// The closure read further by the same logical run (the next document of an explicit
    /// pipeline, a query's projection, a nested closure's consumer): an incomplete closure
    /// is recorded on that run's `truncated` flag, which its later negation gates read.
    fn read_further(self, truncated: &Truncation) -> FactIndex {
        settle(truncated, self.facts)
    }
}

/// The semi-naive forward-chaining fixpoint shared by the id-level and
/// term-level entry points ([`Closure`]).
fn run_closure(
    parsed: parser::Parsed,
    resolver: Option<&Resolver>,
    // The import-cycle guard and runtime-parse allocator ([`VisitedDocs`]). `None` for a
    // TOP-LEVEL run (made fresh against `parsed`); a nested run reached through
    // `formula_closure` passes the PARENT's, so a `log:semantics` / `log:content` document
    // IRI active up the stack is still recognised, and a query or a pipeline stratum passes
    // the one its entry point made.
    // `Some` also means a caller reads this closure further (a query, the next document
    // of an explicit pipeline, a nested closure's consumer).
    inherit: Option<VisitedDocs>,
    // The run's cut record ([`Truncation`]): a fresh one only from a top-level entry
    // point; a nested run passes its parent's, so an inner cut reaches the parent.
    truncated: &Truncation,
    mode: StepMode,
    // What to do with rules on a cycle through negation ([`strata::stratify`]).
    cycles: NegationCycles,
    // The runtime-document table a TOP-LEVEL run draws on when its caller keeps one across
    // runs (a `MaterializedN3Graph`); `None` makes one fresh against `parsed`. Ignored
    // with `inherit`, whose run state already carries its table.
    runtime: Option<&RuntimeDocs>,
) -> Result<Closure, String> {
    validate_rule_quantifiers(&parsed)?;
    // [SONNET-4.6] Rule existentials live in a namespace proven fresh against
    // every blank label in the parsed source, preventing a literal `_:__sk…`
    // from being captured by a minted conclusion blank. Labels ingested later
    // through document builtins or nested closure evaluation are outside this
    // initial-source guarantee.
    let labels = document_blank_labels(&parsed);
    let sk_prefix = fresh_blank_prefix(&labels, "__sk");
    // A caller reads this closure further (a query, the next document of an explicit
    // pipeline, a nested closure's consumer).
    let read_further = inherit.is_some();
    let visited = match inherit {
        Some(v) => v,
        None => run_shared::RunShared::new(match runtime {
            Some(r) => r.clone(),
            // One scan of the document's labels serves both namespaces.
            None => RuntimeDocs::for_labels(&labels),
        }),
    };
    let parser::Parsed { facts: facts0, mut rules, mut backward_rules, base } = parsed;
    // A top-level run keeps terms as deep as those it was handed (terms built in code can
    // nest past the parser's limit); rules cannot build deeper ([`bounded::given_terms`]).
    if !read_further {
        let rule_terms = rules
            .iter()
            .chain(&backward_rules)
            .flat_map(|r| r.premise.iter().chain(&r.conclusion).flatten());
        bounded::given_terms(truncated, facts0.iter().flatten().chain(rule_terms));
    }
    // Premises evaluate left-to-right with no coroutining — reorder each
    // premise so a builtin runs only after the atoms that produce its inputs
    // (cwm evaluates builtins "when ready"; concat.n3 test13f writes the
    // producer AFTER the consumer).
    for r in rules.iter_mut().chain(backward_rules.iter_mut()) {
        r.premise = order_premise(&r.premise);
    }
    // STRATA (GH #6201, #5756): a rule that store-scope negates / aggregates over a
    // predicate this document derives runs in a later stratum than every rule deriving it.
    // A program with no such negation is ONE stratum and takes exactly the single-pass
    // loop below. Rules on a cycle through negation follow `cycles`.
    let strata = strata::stratify(&rules, &backward_rules, cycles)?;
    let mut facts = FactIndex::from_iter(facts0);
    let mut bw = BwCtx::new(&backward_rules, truncated.clone(), visited);
    bw.cycles = cycles;
    bw.base = base;
    bw.resolver = resolver;
    // Derivation steps at the term level (interned to ids once at the end).
    let mut steps: Vec<DerivationStep> = Vec::new();

    // Which predicates can a backward rule conclude? A forward rule with a premise atom on
    // such a predicate cannot use the semi-naive delta restriction (a backward proof is not
    // a fact in any delta), so it re-evaluates fully each round — see `needs_full` below.
    let bw_concl_preds: FxHashSet<&str> = backward_rules
        .iter()
        .flat_map(|r| r.conclusion.iter())
        .filter_map(|c| match &c[1] {
            Term::Iri(i) => Some(i.as_str()),
            _ => None,
        })
        .collect();
    let bw_any_var_pred = backward_rules
        .iter()
        .flat_map(|r| r.conclusion.iter())
        .any(|c| matches!(&c[1], Term::Var(_)));

    // SEMI-NAIVE fixpoint: each round, a positive rule only fires on bindings that involve at
    // least one NEWLY-derived fact (the `delta`) — run once per join-atom position, with that
    // atom restricted to `delta` and the rest to all facts. This avoids re-deriving the whole
    // closure every round (the naive blow-up on recursive rule chains). Rules with scoped
    // negation are non-monotonic, and rules whose join atoms may be proven by BACKWARD rules
    // have support outside the fact deltas — both re-evaluate against ALL facts each round
    // (correct; the fixpoint still terminates because conclusions are deduped). So does every
    // rule whose premise reads the store outside its joins, by its relations' [`StoreRead`]
    // declarations ([`reads_store_outside_joins`]): scoped operators, list walks over a
    // stored list, and virtual list matches, none of which a join delta announces.
    // Pure-builtin rules over rule-local values (no join atom, no store read) fire only in
    // the naive rounds.
    let rule_meta: Vec<(Vec<usize>, bool)> = rules
        .iter()
        .map(|r| {
            let joins: Vec<usize> =
                r.premise.iter().enumerate().filter(|(_, p)| is_join_atom(p)).map(|(i, _)| i).collect();
            let needs_bw = joins.iter().any(|&k| match &r.premise[k][1] {
                Term::Iri(i) => bw_any_var_pred || bw_concl_preds.contains(i.as_str()),
                _ => !backward_rules.is_empty(),
            });
            (joins, needs_bw || reads_store_outside_joins(&r.premise))
        })
        .collect();

    // Conclusion EXISTENTIALS: blank labels in each rule's conclusion (fresh
    // instance per firing), and the conclusion's variables (the firing key —
    // one instantiation per distinct conclusion-relevant binding, so re-runs
    // of non-monotonic rules do not mint endless new blanks).
    let concl_meta: Vec<(Vec<String>, Vec<String>)> =
        rules.iter().map(|r| conclusion_existentials(&r.conclusion)).collect();
    let mut fired: FxHashSet<(usize, String)> = FxHashSet::default();
    let mut sk_counter = 0usize;

    // TRANSITIVITY fast path: rules of the exact shape `{?x P ?y. ?y P ?z} => {?x P ?z}`
    // (ground predicate, three distinct variables, single conclusion, no existentials, not
    // needs_full). The generic semi-naive join is NONLINEAR for these — a new fact joins the
    // FULL P-relation, so on an N-chain every closure pair is re-derived once per intermediate
    // node, O(N³) bindings. Instead evaluate the LINEAR equivalent `R(x,y), GEN(y,z) ⊢ R(x,z)`
    // where GEN is the set of P-edges NOT derived by the transitive rule itself (input facts +
    // facts from other rules): TC(GEN) = TC(R), and each closure pair is derived once per
    // incoming generator edge — O(N²) total. Delta directions: a new fact extends forward
    // through `gen_out` only; a new GENERATOR edge extends every existing path ending at its
    // start backward through the full `po` index. Facts derived by both a transitive rule and
    // another rule may be marked generator or not — both are sound (GEN ⊆ R) and complete
    // (every fact with no transitive-rule derivation is marked).
    struct TransState {
        pred: Term,                          // the ground predicate P
        gen_out: FxHashMap<Term, Vec<Term>>, // generator edges: subject -> objects
        gen_set: FxHashSet<[Term; 3]>,       // generator membership
    }
    let mut trans_states: Vec<TransState> = Vec::new();
    let mut trans_rules: FxHashMap<usize, usize> = FxHashMap::default(); // rule -> state index
    for (ri, rule) in rules.iter().enumerate() {
        let (joins, needs_full) = &rule_meta[ri];
        if *needs_full
            || rule.premise.len() != 2
            || joins.len() != 2
            || rule.conclusion.len() != 1
            || !concl_meta[ri].0.is_empty()
        {
            continue;
        }
        // Accept the two premise atoms in either textual order.
        let detect = |a: &[Term; 3], b: &[Term; 3]| -> Option<(String, String)> {
            match (&a[0], &a[1], &a[2], &b[0], &b[1], &b[2]) {
                (
                    Term::Var(x),
                    Term::Iri(p1),
                    Term::Var(y1),
                    Term::Var(y2),
                    Term::Iri(p2),
                    Term::Var(z),
                ) if p1 == p2 && y1 == y2 && x != y1 && y1 != z && x != z => {
                    Some((x.clone(), z.clone()))
                }
                _ => None,
            }
        };
        let (xz, pred) = match detect(&rule.premise[0], &rule.premise[1]) {
            Some(m) => (m, rule.premise[0][1].clone()),
            None => match detect(&rule.premise[1], &rule.premise[0]) {
                Some(m) => (m, rule.premise[0][1].clone()),
                None => continue,
            },
        };
        let c = &rule.conclusion[0];
        let is_var = |t: &Term, n: &str| matches!(t, Term::Var(v) if v == n);
        if c[1] == pred && is_var(&c[0], &xz.0) && is_var(&c[2], &xz.1) {
            let si = match trans_states.iter().position(|st| st.pred == pred) {
                Some(i) => i,
                None => {
                    trans_states.push(TransState {
                        pred,
                        gen_out: FxHashMap::default(),
                        gen_set: FxHashSet::default(),
                    });
                    trans_states.len() - 1
                }
            };
            trans_rules.insert(ri, si);
        }
    }
    // Seed the generators: every input P-edge.
    for st in trans_states.iter_mut() {
        for f in &facts.all {
            if f[1] == st.pred && st.gen_set.insert(f.clone()) {
                st.gen_out.entry(f[0].clone()).or_default().push(f[2].clone());
            }
        }
    }

    // Which strata close with a naive round in every build: those whose facts a later
    // negation or aggregation reads, so that the gate's "complete store" premise does not
    // rest on the scheduler's classification. A later rule that reads the store through a
    // scope (`log:notIncludes`, `log:collectAllIn`, `log:forAllIn`, a store-scoped
    // `log:includes`) marks every earlier stratum; a run whose closure a caller reads
    // further (a query, the next document of an explicit pipeline, a nested closure)
    // marks its last stratum. A positive program with no such reader keeps the
    // semi-naive fast path.
    let scoped = |ri: usize| {
        rules[ri]
            .premise
            .iter()
            .any(|p| matches!(relation(&p[1]).store_read(), StoreRead::Scoped))
    };
    let mut close_naively = vec![false; strata.n_strata];
    for ri in (0..rules.len()).filter(|&ri| scoped(ri)) {
        let st = strata.rule_stratum.as_ref().map_or(0, |rs| rs[ri]);
        if st == strata::DROPPED {
            continue;
        }
        for c in close_naively.iter_mut().take(st) {
            *c = true;
        }
    }
    if read_further {
        if let Some(last) = close_naively.last_mut() {
            *last = true;
        }
    }

    let mut delta: FxHashSet<[Term; 3]> = facts.all.clone(); // round 0: every fact is "new"
    // The closure's round and fact allowance (the run's `ClosureLimits`): a rule set that
    // never saturates (GH #6757) stops with a cut and an error instead of running forever.
    // Its term-size allowance is checked where builtins derive terms (GH #6771).
    let mut allowance = bounded::ClosureRounds::forward(truncated);
    for (stratum, &close_naively) in close_naively.iter().enumerate() {
        if stratum > 0 {
            // Stratum boundary: everything closed so far is "new" to this stratum's rules.
            delta = facts.all.clone();
        }
        let mut first_round = true;
        // The current round is the stratum's closing naive round.
        let mut closing_check = false;
        loop {
            bounded::ClosureRounds::round(&mut allowance, facts.all.len(), truncated)?;
            bw.work.round();
            // A nested closure failed, or the negation gate refused, in an earlier round:
            // the run fails with that error at the end anyway, so stop here instead of
            // re-evaluating (a failed nested closure is retried every round, and each
            // retry re-runs every level under it).
            if let Some(e) = bw.nested_error.take() {
                return Err(e);
            }
            // A round with `first_round` set is NAIVE: every rule over the whole fact set.
            let naive_round = first_round;
            let mut produced: Vec<DerivationStep> = Vec::new();
            for (ri, rule) in rules.iter().enumerate() {
                if strata
                    .rule_stratum
                    .as_ref()
                    .is_some_and(|rs| rs[ri] != stratum)
                {
                    continue;
                }
                if let Some(&si) = trans_rules.get(&ri) {
                    // Transitivity fast path (linearized; see `TransState` above). Bypasses the
                    // generic binding machinery: the join is two adjacency lookups per delta fact.
                    let st = &trans_states[si];
                    for f in &delta {
                        if f[1] != st.pred {
                            continue;
                        }
                        // forward: Δ ⋈ GEN — extend the new path by generator edges at its end.
                        if let Some(zs) = st.gen_out.get(&f[2]) {
                            // One firing per generator edge, charged before any is built.
                            if !bw.work.charge(zs.len(), truncated) {
                                break;
                            }
                            for z in zs {
                                let g = [f[0].clone(), st.pred.clone(), z.clone()];
                                if !facts.contains(&g) {
                                    let prem = if mode == StepMode::Full {
                                        vec![f.clone(), [f[2].clone(), st.pred.clone(), z.clone()]]
                                    } else {
                                        Vec::new()
                                    };
                                    produced.push((g, ri, prem));
                                }
                            }
                        }
                        // backward: full ⋈ Δgen — a new GENERATOR edge extends every existing
                        // path ending at its start (the po index, incl. same-round delta paths).
                        if st.gen_set.contains(f) {
                            if let Some(xs) = facts.po.get(&(st.pred.clone(), f[0].clone())) {
                                if !bw.work.charge(xs.len(), truncated) {
                                    break;
                                }
                                for x in xs {
                                    let g = [x.clone(), st.pred.clone(), f[2].clone()];
                                    if !facts.contains(&g) {
                                        let prem = if mode == StepMode::Full {
                                            vec![
                                                [x.clone(), st.pred.clone(), f[0].clone()],
                                                f.clone(),
                                            ]
                                        } else {
                                            Vec::new()
                                        };
                                        produced.push((g, ri, prem));
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                let (joins, needs_full) = &rule_meta[ri];
                let bindings: Vec<Binding> = if naive_round || *needs_full || joins.is_empty() {
                    // A naive round, or a rule that reads the store outside its joins
                    // (every round): full evaluation, with no delta seeding. A constant
                    // rule (no join) fires only in naive rounds.
                    if naive_round || *needs_full {
                        match_premise(&rule.premise, &facts, &bw)
                    } else {
                        Vec::new()
                    }
                } else {
                    // Semi-naive: union over delta-at-each-join-position (dedup via facts.insert).
                    let mut bs = Vec::new();
                    for &k in joins {
                        bs.extend(match_premise_seeded(
                            &rule.premise,
                            &facts,
                            &Binding::new(),
                            Some((&delta, k)),
                            &bw,
                            bounded::backward_depth(),
                        ));
                    }
                    bs
                };
                let (concl_blanks, concl_vars) = &concl_meta[ri];
                // The rule's firings, charged before any conclusion is instantiated.
                if !bw.work.charge(bindings.len().saturating_mul(rule.conclusion.len()), truncated) {
                    continue;
                }
                for b in bindings {
                    // Fresh conclusion existentials: rename the conclusion's blanks
                    // once per distinct (rule, conclusion-binding) firing.
                    let sk: Option<FxHashMap<String, String>> = if concl_blanks.is_empty() {
                        None
                    } else {
                        let key: String = concl_vars
                            .iter()
                            .map(|v| format!("{:?};", b.get(v)))
                            .collect();
                        if !fired.insert((ri, key)) {
                            continue; // this firing already instantiated its existentials
                        }
                        sk_counter += 1;
                        Some(
                            concl_blanks
                                .iter()
                                .map(|l| (l.clone(), format!("{}{}_{}", sk_prefix, sk_counter, l)))
                                .collect(),
                        )
                    };
                    for c in &rule.conclusion {
                        let c = match &sk {
                            Some(map) => rename_blanks(c, map),
                            None => c.clone(),
                        };
                        if let Some(g) = ground_triple(&c, &b, &bw.truncated) {
                            // A conclusion past the term-depth limit is dropped, as a cut.
                            if !facts.contains(&g)
                                && bw.settle(bounded::generated(&bw.truncated, &g))
                            {
                                // The supporting facts: premise patterns instantiated under b that
                                // are actual facts (excludes builtins / list structure).
                                let prem: Vec<[Term; 3]> = if mode == StepMode::Full {
                                    rule.premise
                                        .iter()
                                        .filter_map(|p| ground_triple(p, &b, &bw.truncated))
                                        .filter(|t| facts.contains(t))
                                        .collect()
                                } else {
                                    Vec::new()
                                };
                                produced.push((g, ri, prem));
                            }
                        }
                    }
                }
            }
            let mut new_delta: FxHashSet<[Term; 3]> = FxHashSet::default();
            // Generator marking for the transitivity fast path: among this round's NEW facts on a
            // transitive predicate, those with at least one NON-transitive-rule derivation are
            // generators (a fact may be produced by several rules in one round — OR the flags).
            let mut trans_new: FxHashMap<[Term; 3], bool> = FxHashMap::default();
            for (g, ri, prem) in produced {
                let is_new = facts.insert(g.clone());
                if is_new {
                    new_delta.insert(g.clone());
                }
                if !trans_states.is_empty()
                    && (is_new || new_delta.contains(&g))
                    && trans_states.iter().any(|st| st.pred == g[1])
                {
                    *trans_new.entry(g.clone()).or_insert(false) |= !trans_rules.contains_key(&ri);
                }
                if is_new && mode != StepMode::None {
                    steps.push((g, ri, prem));
                }
            }
            for (g, non_trans) in trans_new {
                if !non_trans {
                    continue;
                }
                if let Some(st) = trans_states.iter_mut().find(|st| st.pred == g[1]) {
                    if st.gen_set.insert(g.clone()) {
                        st.gen_out
                            .entry(g[0].clone())
                            .or_default()
                            .push(g[2].clone());
                    }
                }
            }
            first_round = false;
            if new_delta.is_empty() {
                // A stratum that a later negation or aggregation reads closes with a NAIVE
                // round: every rule fully evaluated over the whole fact set, no delta
                // seeding. Anything it derives resumes the loop, so the stratum ends at a
                // true fixpoint whatever the scheduler's classification. Debug builds close
                // EVERY stratum this way and assert the round derives nothing, so a
                // classification gap fails the tests instead of costing a round.
                if (cfg!(debug_assertions) || close_naively) && !naive_round {
                    closing_check = true;
                    first_round = true;
                    delta = facts.all.clone();
                    continue;
                }
                break;
            }
            debug_assert!(
                !closing_check,
                "n3 semi-naive scheduling gap: a naive round derived {} new fact(s), e.g. {:?}",
                new_delta.len(),
                new_delta.iter().next()
            );
            closing_check = false;
            delta = new_delta;
        }
    }
    if let Some(e) = bw.nested_error.take().or_else(|| truncated.refusal()) {
        return Err(e);
    }
    let skipped = strata
        .rule_stratum
        .as_ref()
        .is_some_and(|rs| rs.contains(&strata::DROPPED));
    let facts = if skipped {
        Bounded::cut(
            facts,
            "NegationCycles::FailClosed skipped rules on a negation cycle in an earlier \
             document, so the closure it carries lacks what they derive",
        )
    } else {
        Bounded::complete(facts)
    };
    Ok(Closure {
        facts,
        steps,
        warning: strata.warning,
    })
}

/// Whether [`intern_closure`] builds `N3ProofRun::closure_keys` — a TYPE-level choice, so
/// the plain materialization paths ([`reason_n3`], the stratified closure) do no identity
/// key encoding or allocation for closure rows at all; only [`reason_n3_proof_run`] pays.
trait KeyMode {
    const COLLECT: bool;
}
/// Leave `closure_keys` empty.
enum NoKeys {}
/// Key every closure row.
enum CollectKeys {}
impl KeyMode for NoKeys {
    const COLLECT: bool = false;
}
impl KeyMode for CollectKeys {
    const COLLECT: bool = true;
}

/// Intern a term-level closure + derivation into the dictionary ([`reason_n3`] /
/// [`reason_n3_proof`] output form). Errors on formula-valued facts, which have
/// no dictionary representation (use [`reason_n3_terms`] for those documents).
/// `closure_keys` is filled only under [`CollectKeys`]; each [`ProofStep`] always carries
/// its keys (there are no steps without derivation tracking).
fn intern_closure<K: KeyMode>(
    dict: &mut Dict,
    facts: &FactIndex,
    steps: &[DerivationStep],
) -> Result<N3ProofRun, String> {
    // First-class list values have no dictionary representation — expand them
    // into rdf:first/rest blank-node chains (one chain per list VALUE, shared
    // across the facts that mention it).
    let fact_rows: Vec<[Term; 3]> = facts.all.iter().cloned().collect();
    // The same expander also expands the proof rows below, so its cell labels must be
    // fresh against THOSE blanks too — seed it from both.
    let step_rows = steps.iter().flat_map(|(g, _, prem)| std::iter::once(g).chain(prem));
    let mut exp = ListExpander::new(fact_rows.iter().chain(step_rows));
    // Identity keys from the N3 terms BEFORE list expansion and interning.
    let mut closure_keys: Vec<[String; 3]> = Vec::new();
    if K::COLLECT {
        closure_keys = fact_rows.iter().map(serialize::statement_keys).collect();
    }
    let (fact_rows, mut extra) = exp.expand_rows(&fact_rows);
    if K::COLLECT {
        closure_keys.extend(extra.iter().map(serialize::statement_keys));
    }
    // Intern the ground closure into the dictionary.
    let mut out = Vec::with_capacity(fact_rows.len() + extra.len());
    let mut rows = fact_rows;
    rows.append(&mut extra);
    for t in &rows {
        out.push([intern(dict, &t[0])?, intern(dict, &t[1])?, intern(dict, &t[2])?]);
    }
    // Intern the proof steps (list terms expanded to their chain heads; the
    // chain structure itself is already in the closure rows above).
    let mut proof = Vec::with_capacity(steps.len());
    for (g, ri, prem) in steps {
        let it = |t: &[Term; 3], d: &mut Dict, e: &mut ListExpander| -> Result<[Id; 3], String> {
            let r = e.expand_row(t);
            Ok([intern(d, &r[0])?, intern(d, &r[1])?, intern(d, &r[2])?])
        };
        let conclusion = it(g, dict, &mut exp)?;
        let premises =
            prem.iter().map(|p| it(p, dict, &mut exp)).collect::<Result<Vec<_>, _>>()?;
        proof.push(ProofStep {
            conclusion,
            rule: *ri,
            premises,
            conclusion_key: serialize::statement_keys(g),
            premise_keys: prem.iter().map(serialize::statement_keys).collect(),
        });
    }
    Ok(N3ProofRun { closure: out, closure_keys, steps: proof })
}

/// Expands first-class `Term::List` values into rdf:first/rest blank-node
/// chains for consumers that need pure RDF triples (the dictionary-interning
/// entry points). One chain per distinct list value; `()` becomes `rdf:nil`.
///
/// Cell labels are minted under a `prefix` proven fresh against every blank
/// label reachable in the rows the expander was built for ([`ListExpander::new`]) —
/// a generated cell can therefore never be the same RDF node as a blank the
/// caller is already carrying (a document blank or a projected query answer
/// literally spelled `_:_l1` would otherwise MERGE with a generated cell and
/// emit a semantically wrong graph).
struct ListExpander {
    heads: FxHashMap<Term, Term>,
    structure: Vec<[Term; 3]>,
    counter: usize,
    prefix: String,
}

impl ListExpander {
    /// An expander whose cell labels are fresh against every blank reachable in
    /// `rows` — which must cover EVERY row this expander will be asked to
    /// expand, since the prefix is fixed at construction.
    fn new<'a>(rows: impl IntoIterator<Item = &'a [Term; 3]>) -> Self {
        let mut seen = FxHashSet::default();
        for r in rows {
            r.iter().for_each(|t| term_blank_labels(t, &mut seen));
        }
        let prefix = fresh_blank_prefix(&seen, "_l");
        Self { heads: FxHashMap::default(), structure: Vec::new(), counter: 0, prefix }
    }
    fn expand_rows(&mut self, rows: &[[Term; 3]]) -> (Vec<[Term; 3]>, Vec<[Term; 3]>) {
        let out: Vec<[Term; 3]> = rows.iter().map(|r| self.expand_row(r)).collect();
        (out, std::mem::take(&mut self.structure))
    }
    fn expand_row(&mut self, row: &[Term; 3]) -> [Term; 3] {
        [self.expand(&row[0]), self.expand(&row[1]), self.expand(&row[2])]
    }
    fn expand(&mut self, t: &Term) -> Term {
        // A quoted triple may carry list values in its components; expand them
        // in place (the chain structure is asserted in the OUTER graph) so the
        // triple term itself stays dictionary-representable. [FABLE-5]
        if let Term::Triple(tr) = t {
            return Term::Triple(Box::new([
                self.expand(&tr[0]),
                self.expand(&tr[1]),
                self.expand(&tr[2]),
            ]));
        }
        let Term::List(ms) = t else { return t.clone() };
        if ms.is_empty() {
            return Term::Iri(parser::RDF_NIL.into());
        }
        if let Some(head) = self.heads.get(t) {
            return head.clone();
        }
        let members: Vec<Term> = ms.iter().map(|m| self.expand(m)).collect();
        let mut tail = Term::Iri(parser::RDF_NIL.into());
        for m in members.into_iter().rev() {
            self.counter += 1;
            let node = Term::Blank(format!("{}{}", self.prefix, self.counter));
            self.structure.push([node.clone(), Term::Iri(parser::RDF_FIRST.into()), m]);
            self.structure.push([node.clone(), Term::Iri(parser::RDF_REST.into()), tail]);
            tail = node;
        }
        self.heads.insert(t.clone(), tail.clone());
        tail
    }
}

type Binding = HashMap<String, Term>;

/// All variable bindings under which every premise pattern holds (joining against `facts`,
/// evaluating builtins as filters/computations). N3 collections `( … )` in the premise are
/// rule-local list STRUCTURE (rdf:first/rest over fresh bnodes), not data to match — they are
/// extracted up front and consumed by the functional builtins (e.g. `math:sum`).
fn match_premise(premise: &[[Term; 3]], facts: &FactIndex, bw: &BwCtx) -> Vec<Binding> {
    match_premise_seeded(
        premise,
        facts,
        &Binding::new(),
        None,
        bw,
        bounded::backward_depth(),
    )
}

/// Match `premise` starting from an existing partial binding `seed`. For SEMI-NAIVE
/// evaluation, `delta_at = Some((delta, k))` restricts the join atom at premise index `k` to
/// match only the `delta` set (the newly-derived facts) rather than the full `facts` — so the
/// driver, by running once per join-atom index, considers only bindings that involve ≥1 new
/// fact. `None` = naive (every join atom matches all facts); also used for the negation
/// sub-formula recursion.
///
/// An evaluation boundary ([`within_depth`]): a `seed` past the term-depth limit is
/// refused, as a cut, before any atom is evaluated under it.
fn match_premise_seeded(
    premise: &[[Term; 3]],
    facts: &FactIndex,
    seed: &Binding,
    delta_at: Option<(&FxHashSet<[Term; 3]>, usize)>,
    bw: &BwCtx,
    depth: usize,
) -> Vec<Binding> {
    if !seed.is_empty() && !within_depth(seed, bw) {
        return Vec::new();
    }
    match_premise_admitted(premise, facts, seed, delta_at, bw, depth)
}

/// [`match_premise_seeded`] for a `seed` already within the term-depth limit.
fn match_premise_admitted(
    premise: &[[Term; 3]],
    facts: &FactIndex,
    seed: &Binding,
    delta_at: Option<(&FxHashSet<[Term; 3]>, usize)>,
    bw: &BwCtx,
    depth: usize,
) -> Vec<Binding> {
    // Semi-naive: seed from the DELTA atom first (delta is small → most selective), then
    // evaluate the rest against the index. Doing the delta atom first makes it prune early
    // instead of letting a non-delta atom do a full predicate-index scan.
    if let Some((delta, k)) = delta_at {
        let mut seeds = Vec::new();
        for fact in delta {
            if let Some(nb) = unify(&premise[k], fact, seed, &bw.truncated) {
                if !bw.work.charge(1, &bw.truncated) {
                    return Vec::new();
                }
                seeds.push(nb);
            }
        }
        if seeds.is_empty() {
            return Vec::new();
        }
        let rest: Vec<[Term; 3]> =
            premise.iter().enumerate().filter(|&(i, _)| i != k).map(|(_, p)| p.clone()).collect();
        let mut out = Vec::new();
        // Each seed extends the admitted `seed` with parts of stored facts only.
        for s in &seeds {
            out.extend(match_premise_admitted(&rest, facts, s, None, bw, depth));
        }
        return out;
    }
    let mut bindings: Vec<Binding> = vec![seed.clone()];
    for pat in premise {
        // log:includes / log:notIncludes / log:supports — does the SCOPE
        // include (or entail, for supports) the object formula?
        //   * subject a `{ … }` formula (even `{}` — the EMPTY formula
        //     includes nothing, cwm builtins.n3): SYNTACTIC containment in
        //     that formula. Scope-side quantified terms (`@forAll` variables,
        //     blank existentials) act as OPAQUE CONSTANTS; the PATTERN's
        //     existentials (blanks, `@forSome`) act as wildcards, and its
        //     rule variables bind — exactly cwm's quantifiers_limited matrix.
        //   * log:supports first closes the scope formula under its own
        //     `=>` rules, then checks containment in the closure.
        //   * subject unbound or non-formula: the scope is the current store
        //     (the engine's scoped-negation-as-failure idiom, kept).
        // `log:includes` may BIND free variables of the object pattern (one
        // binding per match, like EYE); `log:notIncludes` holds iff no match.
        if let Some(op) = scope_op(&pat[1]) {
            let inner: &[[Term; 3]] = match &pat[2] {
                Term::Formula(t) => t,
                _ => &[],
            };
            let is_not = matches!(op, ScopeOp::NotIncludes);
            let mut next = Vec::new();
            for b in bindings {
                // A pattern reached through a variable was not visible to
                // [`validate_rule_quantifiers`]: a quantified formula bound to it is refused
                // the same way, failing the run. (Only a pattern written as a formula is
                // matched; any other object matches as the empty pattern.)
                if !matches!(&pat[2], Term::Formula(_)) {
                    if let Term::Var(v) = &pat[2] {
                        if b.get(v).is_some_and(|val| matches!(val, Term::Formula(_)) && quantified_clause(val)) {
                            bw.nested_error.borrow_mut().get_or_insert_with(|| QUANTIFIED_ANTECEDENT.into());
                            continue;
                        }
                    }
                }
                let Some(scope_term) = bounded::substitute_deep(&pat[0], &b, &bw.truncated) else {
                    continue;
                };
                let matches: Vec<Binding> = match scope_term {
                    // log:supports searches the closure in place: no term is built.
                    Term::Formula(ts) if matches!(op, ScopeOp::Supports) => {
                        with_formula_closure(&ts, bw, |scope| formula_containment(scope, inner, &b, bw))
                    }
                    Term::Formula(ts) => formula_containment(&ts, inner, &b, bw),
                    // `{}` parses as the literal true — the EMPTY formula:
                    // it includes nothing (and notIncludes everything).
                    Term::Lit(v, _, _) if v == "true" => formula_containment(NO_STATEMENTS, inner, &b, bw),
                    // The store as the scope: the clause recurses into premise matching,
                    // holding a unit of the run's stack allowance (`_frames`).
                    _ => match bw.settle(bounded::nested_step(&bw.truncated)) {
                        Some(_frames) => match_premise_seeded(inner, facts, &b, None, bw, depth),
                        None => Vec::new(),
                    },
                };
                // Every negation passes the gate, whatever it found.
                if is_not {
                    if negation_gate(bw) && matches.is_empty() {
                        next.push(b);
                    }
                } else {
                    // Each match was charged as it was found: by the containment search
                    // over a formula scope, or by the joins of a store-scoped clause.
                    next.extend(matches.into_iter().filter(|nb| within_depth(nb, bw)));
                }
            }
            bindings = next;
            if bindings.is_empty() {
                break;
            }
            continue;
        }
        // log:collectAllIn / log:forAllIn — scoped aggregation and universal
        // quantification (EYE / N3 builtins spec):
        //   * `( ?template { clause } ?list ) log:collectAllIn ?scope` binds
        //     ?list to the ?template instantiations, one per solution of the
        //     clause in the scope (findall — duplicates kept, in match order;
        //     no solutions ⇒ the empty list).
        //   * `( { clause-a } { clause-b } ) log:forAllIn ?scope` holds iff
        //     EVERY solution of clause-a extends to a solution of clause-b
        //     (vacuously true with none); no clause bindings leak out.
        // The scope follows the log:includes convention: a bound `{ … }`
        // formula is matched syntactically; an unbound or non-formula scope
        // (EYE's idiomatic `?SCOPE` / `_:x`) is the CURRENT STORE, where a
        // clause is full premise evaluation (joins, builtins, backward
        // rules). Both operators are NON-MONOTONIC over a store scope — the
        // solution set can grow as the closure grows, their rules re-evaluate
        // every round (`needs_full`), and derived facts are never retracted —
        // so, exactly like scoped negation, they are only sound over
        // predicates fully present before the stratum starts (`run_closure`
        // stratifies a document automatically; [`reason_n3_stratified`] takes
        // explicit strata). A malformed
        // subject (wrong arity, non-formula clause) FAILS the premise for
        // that binding (fail-closed).
        if let Some(op) = collect_op(&pat[1]) {
            let mut next = Vec::new();
            for b in bindings {
                // Solutions of a `{ clause }` value under `seed`, against the
                // applied scope (`None` ⇒ the clause is not a formula).
                let solve = |clause: &Term, seed: &Binding| -> Option<Vec<Binding>> {
                    let atoms: &[[Term; 3]] = match clause {
                        Term::Formula(ts) => ts,
                        // `{}` parses as the literal true: no constraint —
                        // exactly one (empty) solution.
                        Term::Lit(v, _, _) if v == "true" => &[],
                        _ => return None,
                    };
                    Some(match bounded::substitute_deep(&pat[2], seed, &bw.truncated)? {
                        Term::Formula(scope) => formula_containment(&scope, atoms, seed, bw),
                        Term::Lit(v, _, _) if v == "true" => {
                            formula_containment(NO_STATEMENTS, atoms, seed, bw)
                        }
                        // The store as the scope: as for the scoped operators above.
                        _ => match bw.settle(bounded::nested_step(&bw.truncated)) {
                            Some(_frames) => {
                                match_premise_seeded(atoms, facts, seed, None, bw, depth)
                            }
                            None => Vec::new(),
                        },
                    })
                };
                let Some(subj) = bounded::substitute(&pat[0], &b, &bw.truncated) else { continue };
                let Term::List(members) = &subj else { continue };
                // A clause reached through a variable (the subject, or a list member) was
                // not visible to [`validate_rule_quantifiers`]; its bound value is checked
                // here and refused the same way, failing the run. A clause written as a
                // formula was checked there: a value bound into it is an ordinary term,
                // matched by identity.
                if aggregation_clauses(op, &subj).iter().enumerate().any(|(i, c)| {
                    let written = aggregation_clauses(op, &pat[0]).get(i);
                    !matches!(written, Some(Term::Formula(_) | Term::Lit(..))) && quantified_clause(c)
                }) {
                    bw.nested_error.borrow_mut().get_or_insert_with(|| QUANTIFIED_ANTECEDENT.into());
                    continue;
                }
                match op {
                    CollectOp::CollectAll => {
                        let [template, clause, list_pat] = &members[..] else { continue };
                        let Some(sols) = solve(clause, &b) else { continue };
                        if !negation_gate(bw) {
                            continue;
                        }
                        // One copy of the template per solution, sized before it is built.
                        let bytes = sols
                            .iter()
                            .map(|s| bounded::resolved_bytes(template, s, true))
                            .fold(bounded::TERM_NODE_BYTES, usize::saturating_add);
                        if refused_term(&bw.truncated, &bw.truncated, Derived::Collected, bytes, || bytes) {
                            continue;
                        }
                        let Some(items) = sols
                            .iter()
                            .map(|s| bounded::substitute_deep(template, s, &bw.truncated))
                            .collect::<Option<Vec<Term>>>()
                        else {
                            continue;
                        };
                        let collected = Term::List(items);
                        let mut nb = b.clone();
                        if unify_term(list_pat, &collected, &mut nb, &bw.truncated) && within_depth(&nb, bw) {
                            next.push(nb);
                        }
                    }
                    CollectOp::ForAll => {
                        let [ca, cb] = &members[..] else { continue };
                        let Some(sols_a) = solve(ca, &b) else { continue };
                        let holds = sols_a
                            .iter()
                            .all(|s| matches!(solve(cb, s), Some(ss) if !ss.is_empty()));
                        if negation_gate(bw) && holds {
                            next.push(b);
                        }
                    }
                }
            }
            bindings = next;
            if bindings.is_empty() {
                break;
            }
            continue;
        }
        if let Some(gen) = list_generator(&pat[1]) {
            // list:member / list:in / list:iterate — one binding per member.
            let (list_pos, var_pos) = match gen {
                ListGen::Member | ListGen::Iterate => (&pat[0], &pat[2]),
                ListGen::In => (&pat[2], &pat[0]),
            };
            let mut next = Vec::new();
            for b in &bindings {
                let Some(head) = bounded::substitute(list_pos, b, &bw.truncated) else { continue };
                // A first-class `( … )` list value, hand-written rdf:first/rest
                // rule structure, or a data list reached through a bound
                // variable (walked from the fact store).
                let members: Option<std::borrow::Cow<'_, [Term]>> = match &head {
                    Term::List(ms) => Some(std::borrow::Cow::Borrowed(&ms[..])),
                    _ => bw.settle(fact_list(&head, facts)).map(std::borrow::Cow::Owned),
                };
                if let Some(members) = members {
                    for (ix, m) in members.iter().enumerate() {
                        // The term this binds, sized from the borrowed member first: the
                        // member, or for list:iterate an (index member) pair.
                        let mut bytes = bounded::resolved_bytes(m, b, false);
                        if let ListGen::Iterate = gen {
                            let index = Numeral::count(ix).term_bytes();
                            bytes = bytes.saturating_add(bounded::TERM_NODE_BYTES).saturating_add(index);
                        }
                        if refused_term(&bw.truncated, &bw.truncated, Derived::Generated(gen), bytes, || bytes) {
                            continue;
                        }
                        let Some(mv) = bounded::substitute(m, b, &bw.truncated) else { continue };
                        let target = match gen {
                            ListGen::Member | ListGen::In => mv,
                            // (?index ?value) pairs, 0-based (EYE list:iterate).
                            ListGen::Iterate => Term::List(vec![
                                Term::Lit(ix.to_string(), parser::XSD_INTEGER.into(), None),
                                mv,
                            ]),
                        };
                        let mut nb = b.clone();
                        if unify_term(var_pos, &target, &mut nb, &bw.truncated) && within_depth(&nb, bw) {
                            if !bw.work.charge(1, &bw.truncated) {
                                return Vec::new();
                            }
                            next.push(nb);
                        }
                    }
                }
            }
            bindings = next;
        } else if let Some(f) = functional_builtin(&pat[1]) {
            bindings = bindings
                .into_iter()
                .filter_map(|b| bw.settle(eval_functional(f, &pat[0], &pat[2], facts, bw, b)))
                .filter(|nb| within_depth(nb, bw))
                .collect();
        } else if let Some(op) = binder_builtin(&pat[1]) {
            bindings = bindings
                .into_iter()
                .filter_map(|b| bw.settle(eval_binder(op, &pat[0], &pat[2], b, &bw.truncated)))
                .filter(|nb| within_depth(nb, bw))
                .collect();
        } else if let Some(op) = builtin(&pat[1]) {
            bindings.retain(|b| bw.settle(eval_builtin(op, &pat[0], &pat[2], b, &bw.truncated)));
        } else {
            // Join atom: selective FactIndex lookup (no full scan) for each current binding,
            // PLUS goal-directed resolution against the backward (`<=`) rules.
            let mut next = Vec::new();
            for b in &bindings {
                let run = &bw.truncated;
                let (Some(s_a), Some(p_a), Some(o_a)) = (
                    bounded::substitute(&pat[0], b, run),
                    bounded::substitute(&pat[1], b, run),
                    bounded::substitute(&pat[2], b, run),
                ) else {
                    continue;
                };
                // Virtual rdf:first / rdf:rest over a first-class list value —
                // cwm/EYE expose list structure to matching even though the
                // list is a term, not triples (`?L rdf:first ?X` computes).
                if let (Term::List(ms), Term::Iri(pi)) = (&s_a, &p_a) {
                    if pi == parser::RDF_FIRST || pi == parser::RDF_REST {
                        if !ms.is_empty() {
                            // The tail is sized before it is built.
                            let val = if pi == parser::RDF_FIRST {
                                Some(ms[0].clone())
                            } else {
                                bounded::list_tail(ms, run)
                            };
                            let mut nb = b.clone();
                            if val.is_some_and(|val| unify_term(&pat[2], &val, &mut nb, &bw.truncated)) && within_depth(&nb, bw) {
                                if !bw.work.charge(1, run) {
                                    return Vec::new();
                                }
                                next.push(nb);
                            }
                        }
                        continue; // a list term is never the subject of stored first/rest triples
                    }
                }
                let cands = facts.candidates(&s_a, &p_a, &o_a);
                for fact in &cands {
                    if let Some(nb) = unify(pat, fact, b, &bw.truncated) {
                        // Each row is charged before it is kept ([`bounded::JoinWork`]).
                        if !bw.work.charge(1, run) {
                            return Vec::new();
                        }
                        next.push(nb);
                    }
                }
                if !bw.rules.is_empty() {
                    let could_match =
                        bw.rules.iter().flat_map(|r| &r.conclusion).any(
                            |c| !matches!((&p_a, &c[1]), (Term::Iri(a), Term::Iri(b)) if a != b),
                        );
                    let step = bounded::backward_step(&bw.truncated, depth, could_match);
                    // `_frames` holds the step's unit of the run's stack allowance.
                    if let Some((d, _frames)) = bw.settle(step) {
                        // Each proof was charged as [`backward_prove`] built it.
                        next.extend(
                            backward_prove(pat, b, facts, bw, d)
                                .into_iter()
                                .filter(|nb| within_depth(nb, bw)),
                        );
                    }
                }
            }
            bindings = next;
        }
        if bindings.is_empty() {
            break;
        }
    }
    bindings
}

/// Whether every value `b` binds is within the term-depth limit
/// ([`bounded::generated`]). A binding an atom built past it is dropped, as a cut: the
/// premise atoms that can build a deeper value than they read (builtins, list
/// generators, virtual list access, scoped operators, backward proofs) filter their
/// bindings through here, so every binding a premise carries forward is within it. The
/// containment search of a formula scope ([`containment_search`]) checks each binding it
/// extends here too, before carrying it to its next statement.
///
/// The invariant: nothing past the limit enters an evaluation. Each recursive evaluation
/// boundary checks what it is handed before it evaluates anything, and refuses it as a
/// cut:
/// - premise evaluation from a seed ([`match_premise_seeded`]): the seed, which covers a
///   backward proof step's substitution, a store-scoped clause's binding and a
///   `log:collectAllIn` / `log:forAllIn` clause's binding;
/// - a backward proof step ([`backward_prove`]): its instantiated goal;
/// - a formula-scope search ([`formula_containment`]): its seed and the scope;
/// - a nested closure ([`formula_closure`], `log:supports` / `log:conclusion`): the
///   statements it would admit as facts and rules.
///
/// A top-level run's own input is admitted by raising the limit to it
/// ([`bounded::given_terms`]); parsed text is within the parser's nesting limit.
fn within_depth(b: &Binding, bw: &BwCtx) -> bool {
    bw.settle(bounded::generated(&bw.truncated, b.values()))
}

/// Whether a premise reads the fact store outside its join atoms, from each relation's
/// [`StoreRead`] declaration. Such a rule re-evaluates every round: what it reads can be
/// derived (or, for list terms, exist) without appearing in any round's delta.
fn reads_store_outside_joins(premise: &[[Term; 3]]) -> bool {
    premise.iter().any(reads_outside_joins)
}

/// [`reads_store_outside_joins`] for one premise atom.
fn reads_outside_joins(p: &[Term; 3]) -> bool {
    let not_a_list_term = |t: &Term| !matches!(t, Term::List(_));
    match relation(&p[1]).store_read() {
        StoreRead::Nothing | StoreRead::Joins => false,
        StoreRead::ListCells(Operand::Subject) => not_a_list_term(&p[0]),
        StoreRead::ListCells(Operand::Object) => not_a_list_term(&p[2]),
        StoreRead::MemberListCells | StoreRead::Nested | StoreRead::Scoped => true,
        // A constant subject that is not a list term matches stored triples only.
        StoreRead::VirtualList => matches!(&p[0], Term::Var(_) | Term::Blank(_) | Term::List(_)),
    }
}

/// Stable-reorder a premise so each builtin atom comes after the atoms that
/// can produce its input variables. Join atoms are always "ready" and keep
/// their relative order; a builtin whose inputs are not yet available is
/// deferred. If nothing is ready (e.g. the unbound-scope negation idiom) the
/// first remaining atom runs, preserving the legacy order.
fn order_premise(premise: &[[Term; 3]]) -> Vec<[Term; 3]> {
    fn term_vars(t: &Term, out: &mut FxHashSet<String>) {
        match t {
            Term::Var(v) => {
                out.insert(v.clone());
            }
            Term::List(ms) => ms.iter().for_each(|m| term_vars(m, out)),
            Term::Formula(ts) => ts
                .iter()
                .for_each(|r| r.iter().for_each(|m| term_vars(m, out))),
            Term::Triple(tr) => tr.iter().for_each(|m| term_vars(m, out)),
            _ => {}
        }
    }
    let vars_of = |t: &Term| {
        let mut s = FxHashSet::default();
        term_vars(t, &mut s);
        s
    };
    let mut remaining: Vec<usize> = (0..premise.len()).collect();
    let mut produced: FxHashSet<String> = FxHashSet::default();
    let mut out: Vec<[Term; 3]> = Vec::new();
    while !remaining.is_empty() {
        let ready = |i: usize| -> bool {
            let pat = &premise[i];
            if is_join_atom(pat) {
                return true;
            }
            let subj_ready = vars_of(&pat[0]).is_subset(&produced);
            let obj_ready = vars_of(&pat[2]).is_subset(&produced);
            if builtin(&pat[1]).is_some() {
                return subj_ready && obj_ready; // comparison: both are inputs
            }
            if let Some(gen) = list_generator(&pat[1]) {
                return match gen {
                    ListGen::Member | ListGen::Iterate => subj_ready,
                    ListGen::In => obj_ready,
                };
            }
            // functional / binder / scope op: bidirectional ops accept either
            // side; the rest need the subject.
            let bidi = matches!(
                functional_builtin(&pat[1]),
                Some(Func::Dtlit | Func::Negation | Func::InSeconds)
            ) || binder_builtin(&pat[1]).is_some();
            if bidi {
                subj_ready || obj_ready
            } else {
                subj_ready
            }
        };
        let pos = remaining.iter().position(|&i| ready(i)).unwrap_or(0);
        let i = remaining.remove(pos);
        for t in &premise[i] {
            term_vars(t, &mut produced); // its outputs are now available
        }
        out.push(premise[i].clone());
    }
    out
}

/// Whether a premise pattern is a JOIN atom (matched against stored facts), by its
/// relation's [`StoreRead`] declaration. A virtual relation is a join unless its subject is
/// written as a list term (then it only computes over that term).
fn is_join_atom(pat: &[Term; 3]) -> bool {
    match relation(&pat[1]).store_read() {
        StoreRead::Joins => true,
        StoreRead::VirtualList => !matches!(&pat[0], Term::List(_)),
        StoreRead::Nothing
        | StoreRead::ListCells(_)
        | StoreRead::MemberListCells
        | StoreRead::Nested
        | StoreRead::Scoped => false,
    }
}

/// Goal-directed (`<=`) resolution of one premise atom: for each backward rule whose
/// conclusion unifies with the goal (under the current binding `b`), prove that rule's
/// premise — recursively allowing joins, builtins, and further backward rules — and project
/// each proof back onto the goal's variables. SLD resolution with standardized-apart rule
/// variables and a depth bound (`depth` counts REMAINING backward applications).
fn backward_prove(pat: &[Term; 3], b: &Binding, facts: &FactIndex, bw: &BwCtx, depth: usize) -> Vec<Binding> {
    let run = &bw.truncated;
    let (Some(g0), Some(g1), Some(g2)) = (
        bounded::substitute(&pat[0], b, run),
        bounded::substitute(&pat[1], b, run),
        bounded::substitute(&pat[2], b, run),
    ) else {
        return Vec::new();
    };
    let goal = [g0, g1, g2];
    // An evaluation boundary ([`within_depth`]): a goal past the term-depth limit is not
    // resolved. Each rule's substitution is checked where its premise is evaluated
    // ([`match_premise_seeded`]).
    if !bw.settle(bounded::generated(&bw.truncated, &goal)) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for rule in bw.rules {
        for concl in &rule.conclusion {
            // Cheap gate: ground predicates must agree before we rename/unify anything.
            if let (Term::Iri(gp), Term::Iri(cp)) = (&goal[1], &concl[1]) {
                if gp != cp {
                    continue;
                }
            }
            // Standardize apart: fresh names for this rule application's variables.
            let n = bw.rename.get();
            bw.rename.set(n + 1);
            let rc = [rename_vars(&concl[0], n), rename_vars(&concl[1], n), rename_vars(&concl[2], n)];
            let mut subst = b.clone();
            if !(unify_walked(&goal[0], &rc[0], &mut subst)
                && unify_walked(&goal[1], &rc[1], &mut subst)
                && unify_walked(&goal[2], &rc[2], &mut subst))
            {
                continue;
            }
            let prem: Vec<[Term; 3]> = rule
                .premise
                .iter()
                .map(|t| [rename_vars(&t[0], n), rename_vars(&t[1], n), rename_vars(&t[2], n)])
                .collect();
            for sol in match_premise_seeded(&prem, facts, &subst, None, bw, depth) {
                // Project the proof onto the goal's variables (walk chains like
                // outer-var → renamed-var → ground value).
                let mut nb = b.clone();
                let mut ok = true;
                for g in pat {
                    if let Term::Var(_) = g {
                        // Deep-resolve: walk the chain, then substitute inside
                        // any list structure the value carries.
                        let Some(val) = bounded::substitute(&walk(g, &sol), &sol, &bw.truncated)
                        else {
                            ok = false;
                            break;
                        };
                        if (val.is_ground() || matches!(val, Term::Formula(_)))
                            && !unify_term(g, &val, &mut nb, &bw.truncated)
                        {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    // Each proof is a row of the goal's atom, charged before it is kept.
                    if !bw.work.charge(1, &bw.truncated) {
                        return Vec::new();
                    }
                    out.push(nb);
                }
            }
        }
    }
    out
}

/// `t` with every variable renamed into the standardize-apart space of rule application `n`
/// (recursing into quoted formulae).
fn rename_vars(t: &Term, n: usize) -> Term {
    match t {
        // A quoted formula's own universal is not a rule variable (GH #6754).
        Term::Var(v) if quoted_universal(v) => t.clone(),
        Term::Var(v) => Term::Var(format!("__bw{n}_{v}")),
        Term::Formula(ts) => Term::Formula(
            ts.iter().map(|tr| [rename_vars(&tr[0], n), rename_vars(&tr[1], n), rename_vars(&tr[2], n)]).collect(),
        ),
        Term::List(ms) => Term::List(ms.iter().map(|m| rename_vars(m, n)).collect()),
        Term::Triple(tr) => Term::Triple(Box::new([
            rename_vars(&tr[0], n),
            rename_vars(&tr[1], n),
            rename_vars(&tr[2], n),
        ])),
        _ => t.clone(),
    }
}

/// Resolve `t` through any chain of variable bindings in `s` (bounded against cycles).
fn walk(t: &Term, s: &Binding) -> Term {
    let mut cur = t.clone();
    for _ in 0..s.len() + 1 {
        match &cur {
            Term::Var(v) => match s.get(v) {
                Some(next) => cur = next.clone(),
                None => break,
            },
            _ => break,
        }
    }
    cur
}

/// Unification where BOTH sides may contain variables (goal ↔ backward-rule conclusion),
/// chain-resolving each side through `s` first. Unbound-vs-unbound binds left → right, so a
/// free goal variable points AT the rule variable and is grounded by the premise proof.
fn unify_walked(a: &Term, c: &Term, s: &mut Binding) -> bool {
    let aw = walk(a, s);
    let cw = walk(c, s);
    if aw == cw {
        return true;
    }
    match (&aw, &cw) {
        (Term::Var(v), _) if !unbindable_var(v) => {
            if frees_quoted_universal(&cw) {
                return false;
            }
            s.insert(v.clone(), cw.clone());
            true
        }
        (_, Term::Var(v)) if !unbindable_var(v) => {
            if frees_quoted_universal(&aw) {
                return false;
            }
            s.insert(v.clone(), aw.clone());
            true
        }
        // Structural list unification, member by member (either side may hold
        // variables — backward goals over list arguments).
        (Term::List(xs), Term::List(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| unify_walked(x, y, s))
        }
        // Structural quoted-triple unification (backward goals over
        // quoted-triple arguments). [FABLE-5]
        (Term::Triple(xs), Term::Triple(ys)) => {
            xs.iter().zip(ys.iter()).all(|(x, y)| unify_walked(x, y, s))
        }
        _ => false,
    }
}

/// Walk an rdf:first/rest list that lives in the FACT store (a collection
/// asserted in the data, reached through a bound variable) — the complement of
/// [`extract_lists`], which resolves rule-local structure. `rdf:nil` is the
/// empty list.
fn fact_list(head: &Term, facts: &FactIndex) -> Bounded<Option<Vec<Term>>> {
    fact_list_refs(head, facts).map(|ms| ms.map(|ms| ms.into_iter().cloned().collect()))
}

/// [`fact_list`], borrowing the members from the store (so they can be sized before
/// they are copied).
fn fact_list_refs<'f>(head: &Term, facts: &'f FactIndex) -> Bounded<Option<Vec<&'f Term>>> {
    let first = Term::Iri(parser::RDF_FIRST.into());
    let rest = Term::Iri(parser::RDF_REST.into());
    let nil = Term::Iri(parser::RDF_NIL.into());
    let empty = Term::List(Vec::new());
    bounded::walk_list(
        head.clone(),
        |cur| *cur == nil || *cur == empty,
        |cur| {
            let f = facts.ps.get(&(first.clone(), cur.clone()))?.first()?;
            let r = facts.ps.get(&(rest.clone(), cur.clone()))?.first()?.clone();
            Some((f, r))
        },
    )
}

/// Rename blank labels per `map` in a conclusion triple (recursing into lists;
/// quoted formulae keep their own existentials as written).
fn rename_blanks(t: &[Term; 3], map: &FxHashMap<String, String>) -> [Term; 3] {
    fn go(t: &Term, map: &FxHashMap<String, String>) -> Term {
        match t {
            Term::Blank(l) => match map.get(l) {
                Some(nl) => Term::Blank(nl.clone()),
                None => t.clone(),
            },
            Term::List(ms) => Term::List(ms.iter().map(|m| go(m, map)).collect()),
            Term::Triple(tr) => {
                Term::Triple(Box::new([go(&tr[0], map), go(&tr[1], map), go(&tr[2], map)]))
            }
            _ => t.clone(),
        }
    }
    [go(&t[0], map), go(&t[1], map), go(&t[2], map)]
}

/// Try to unify pattern triple `pat` with ground fact `f`, extending binding `b`. When the
/// unification runs out of steps it fails, and the cut is recorded on `run`.
fn unify(pat: &[Term; 3], f: &[Term; 3], b: &Binding, run: &Truncation) -> Option<Binding> {
    let mut nb = b.clone();
    let mut steps = bounded::StepBudget::unification();
    match unify_all(pat, f, &mut nb, &mut steps) {
        Some(true) => Some(nb),
        Some(false) => None,
        None => {
            bounded::unification_exhausted(run);
            None
        }
    }
}

/// Is `v` a quoted formula's own `@forAll` universal (GH #6754)? Such a variable is bound
/// inside its formula, so matching treats it as a constant (copies of one closed formula
/// carry the same name) and no rule substitution ever reaches it.
fn quoted_universal(v: &str) -> bool {
    v.starts_with(serialize::FORMULA_UNIVERSAL)
}

/// A variable no matcher may bind: a quoted formula's universal, or a rule premise's own
/// universal (`__up.`), which [`validate_rule_quantifiers`] rejects before any matching.
/// Should one reach a matcher anyway, it matches only itself instead of binding as an
/// ordinary variable (and a debug build stops).
fn unbindable_var(v: &str) -> bool {
    if v.starts_with(serialize::PREMISE_UNIVERSAL) {
        debug_assert!(false, "premise universal {v} reached a matcher past validate_rule_quantifiers");
        return true;
    }
    quoted_universal(v)
}

/// Would binding a rule variable to `t` take a quoted formula's universal out of the formula
/// that binds it — does `t` mention one whose declaring formula is not inside `t`?
fn frees_quoted_universal(t: &Term) -> bool {
    fn go(t: &Term, depth: usize) -> bool {
        match t {
            Term::Var(v) => serialize::formula_universal(v).is_some_and(|(n, _)| n >= depth),
            Term::Formula(ts) => ts.iter().flatten().any(|m| go(m, depth + 1)),
            Term::List(ms) => ms.iter().any(|m| go(m, depth)),
            Term::Triple(tr) => tr.iter().any(|m| go(m, depth)),
            _ => false,
        }
    }
    go(t, 0)
}

/// The refusal for a rule antecedent no engine can match soundly (GH #6754; support is
/// tracked in GH #6882).
const QUANTIFIED_ANTECEDENT: &str = "N3: universally quantified antecedents are unsupported (@forAll in a rule premise or scoped body pattern)";

/// The matching clauses of a `log:collectAllIn` / `log:forAllIn` subject list: the
/// clause of `( template clause list )`, both clauses of `( clause-a clause-b )`. The
/// template and the result list are values, not clauses. A subject of another shape
/// yields none (its evaluation fails the premise anyway).
fn aggregation_clauses(op: CollectOp, subject: &Term) -> &[Term] {
    match (op, subject) {
        (CollectOp::CollectAll, Term::List(ms)) if ms.len() == 3 => &ms[1..2],
        (CollectOp::ForAll, Term::List(ms)) if ms.len() == 2 => &ms[..],
        _ => &[],
    }
}

/// The one walk behind every antecedent-quantifier check (GH #6754): does an EXECUTABLE
/// formula — a rule premise, an aggregation clause, a containment pattern, at any nesting
/// depth — bind a universal that occurs anywhere below it? Matching such a formula would
/// need quantified (wildcard) matching, which no engine does soundly.
///
/// `levels` holds the formula levels from the walk's root to the current one, innermost
/// last: whether each is executed (`true`) or a value. A position is executable only
/// where the matcher runs it: the object pattern of `log:includes` / `notIncludes` /
/// `supports`, the clause of `( template clause list ) log:collectAllIn`, and both clauses
/// of `log:forAllIn` — each inside an executable conjunction. Everything else (an
/// aggregation template, a scope, an ordinary quoted formula term, and every atom of a
/// value) is a value, matched by identity, so its own universals are allowed while those
/// of an enclosing executable formula are not. A universal whose binder lies past the
/// root, or a malformed one, counts as quantified (fail-closed).
struct QuantifierWalk {
    levels: Vec<bool>,
}

impl QuantifierWalk {
    /// The premise `atoms` of a rule (an executable root; its own universals are the
    /// rule's `__up.` markers).
    fn premise(atoms: &[[Term; 3]]) -> bool {
        QuantifierWalk { levels: vec![true] }.conjunction(atoms)
    }

    /// A term bound into an executable position at run time: a formula there is a clause.
    fn bound_clause(t: &Term) -> bool {
        QuantifierWalk { levels: Vec::new() }.term(t, true)
    }

    /// An executable conjunction at the current level.
    fn conjunction(&mut self, atoms: &[[Term; 3]]) -> bool {
        atoms.iter().any(|[s, p, o]| {
            if scope_op(p).is_some() {
                self.term(s, false) || self.term(p, false) || self.term(o, true)
            } else if let Some(op) = collect_op(p) {
                let subject = match s {
                    Term::List(ms) => {
                        let clauses = aggregation_clauses(op, s);
                        ms.iter().any(|m| self.term(m, clauses.iter().any(|c| std::ptr::eq(c, m))))
                    }
                    other => self.term(other, false),
                };
                subject || self.term(p, false) || self.term(o, false)
            } else {
                [s, p, o].into_iter().any(|t| self.term(t, false))
            }
        })
    }

    /// `t` at the current level: a formula in an executable position is a clause.
    fn term(&mut self, t: &Term, executable: bool) -> bool {
        match t {
            Term::Var(v) => self.bound_by_executable(v),
            Term::Formula(ts) => {
                self.levels.push(executable);
                let found = if executable {
                    self.conjunction(ts)
                } else {
                    ts.iter().flatten().any(|m| self.term(m, false))
                };
                self.levels.pop();
                found
            }
            Term::List(ms) => ms.iter().any(|m| self.term(m, false)),
            Term::Triple(tr) => tr.iter().any(|m| self.term(m, false)),
            _ => false,
        }
    }

    /// Whether the universal `v` names is bound by an executable level.
    fn bound_by_executable(&self, v: &str) -> bool {
        if v.starts_with(serialize::PREMISE_UNIVERSAL) {
            // only a rule premise binds one, and it is executable
            return true;
        }
        if !v.starts_with(serialize::FORMULA_UNIVERSAL) {
            return false;
        }
        let Some((n, _)) = serialize::formula_universal(v) else { return true };
        self.levels.len().checked_sub(n + 1).is_none_or(|binder| self.levels[binder])
    }
}

/// Whether a clause bound at run time (an aggregation clause or a containment pattern
/// reached through a variable) would need quantified matching ([`QuantifierWalk`]).
fn quantified_clause(clause: &Term) -> bool {
    QuantifierWalk::bound_clause(clause)
}

/// Reject antecedent quantifiers that the rule matchers cannot soundly evaluate. This
/// sees the clauses as written, at every nesting depth; a clause reached through a variable
/// is checked again on its bound value when it is evaluated ([`match_premise_seeded`]),
/// by the same walk.
pub(crate) fn validate_rule_quantifiers(parsed: &parser::Parsed) -> Result<(), String> {
    for rule in parsed.rules.iter().chain(&parsed.backward_rules) {
        if QuantifierWalk::premise(&rule.premise) {
            return Err(QUANTIFIED_ANTECEDENT.into());
        }
    }
    Ok(())
}

/// [`unify_term_in`] under a fresh [`bounded::StepBudget::unification`]. When the budget
/// runs out the terms do not unify, and the cut is recorded on `run`.
fn unify_term(pat: &Term, val: &Term, b: &mut Binding, run: &Truncation) -> bool {
    let mut steps = bounded::StepBudget::unification();
    unify_term_in(pat, val, b, &mut steps).unwrap_or_else(|| {
        bounded::unification_exhausted(run);
        false
    })
}

/// Unify `pat` with `val`, extending `b`. Matching two quoted formulae takes steps from
/// `steps` ([`formula_unify`]); `None` when they run out.
fn unify_term_in(
    pat: &Term,
    val: &Term,
    b: &mut Binding,
    steps: &mut bounded::StepBudget,
) -> Option<bool> {
    Some(match (pat, val) {
        (Term::Var(v), _) if unbindable_var(v) => pat == val,
        (Term::Var(v), _) => match b.get(v) {
            Some(existing) => existing == val,
            None => {
                if !v.starts_with(PATTERN_WILDCARD) && frees_quoted_universal(val) {
                    return Some(false);
                }
                b.insert(v.clone(), val.clone());
                true
            }
        },
        // First-class lists unify STRUCTURALLY: same length, members pairwise
        // (so `(?x)` matches `(17)` binding ?x=17 — cwm/EYE list unification).
        (Term::List(ps), Term::List(vs)) => {
            ps.len() == vs.len() && unify_all(ps, vs, b, steps)?
        }
        // Unification THROUGH quoting: a `{ … }` pattern matches a `{ … }`
        // value when their triple multisets correspond under the binding
        // (pattern variables may bind quoted terms — cwm unify1/unify2).
        (Term::Formula(ps), Term::Formula(vs)) => formula_unify(ps, vs, b, steps)?,
        // Quoted triples unify STRUCTURALLY, component by component (so
        // `<< ?s :p ?o >>` matches `<< :a :p :b >>` binding ?s/?o —
        // SPARQL-star / RDF-star triple-pattern semantics). [FABLE-5]
        (Term::Triple(ps), Term::Triple(vs)) => unify_all(&ps[..], &vs[..], b, steps)?,
        (other, _) => other == val,
    })
}

/// [`unify_term_in`] pairwise over `ps` and `vs` (of equal length), stopping at the first
/// pair that does not unify.
fn unify_all(
    ps: &[Term],
    vs: &[Term],
    b: &mut Binding,
    steps: &mut bounded::StepBudget,
) -> Option<bool> {
    for (p, v) in ps.iter().zip(vs) {
        if !unify_term_in(p, v, b, steps)? {
            return Some(false);
        }
    }
    Some(true)
}

/// Multiset unification of two formula bodies under `b`: a backtracking search that
/// matches pattern statement `i` to an unused value statement, with a binding clone per
/// branch. The search state is an explicit stack (one entry per matched statement), so
/// formulae of any width take no stack per statement.
///
/// Each pairing it tries takes a step from `steps`. The outermost formula match gets a
/// free allowance of [`formula_statements`]`(ps) * formula_statements(vs)` steps, which
/// covers every pairing of a search that never backtracks, nested matches included (it
/// tries each pair of statements at most once), so only backtracking spends the budget.
/// `None` when the steps run out.
fn formula_unify(
    ps: &[[Term; 3]],
    vs: &[[Term; 3]],
    b: &mut Binding,
    steps: &mut bounded::StepBudget,
) -> Option<bool> {
    if ps.len() != vs.len() {
        return Some(false);
    }
    steps.enter_formula(formula_statements(ps).saturating_mul(formula_statements(vs)));
    let matched = formula_search(ps, vs, b, steps);
    steps.exit_formula();
    matched
}

/// The statements of `ts` and of every formula nested in them.
fn formula_statements(ts: &[[Term; 3]]) -> usize {
    let mut n = ts.len();
    let mut stack: Vec<&Term> = ts.iter().flatten().collect();
    while let Some(t) = stack.pop() {
        match t {
            Term::Formula(rs) => {
                n = n.saturating_add(rs.len());
                stack.extend(rs.iter().flatten());
            }
            Term::List(ms) => stack.extend(ms),
            Term::Triple(tr) => stack.extend(tr.iter()),
            _ => {}
        }
    }
    n
}

/// The search of [`formula_unify`].
fn formula_search(
    ps: &[[Term; 3]],
    vs: &[[Term; 3]],
    b: &mut Binding,
    steps: &mut bounded::StepBudget,
) -> Option<bool> {
    let mut used = vec![false; vs.len()];
    // `stack[i]`: the binding after matching `ps[..i]`, and the next value statement to
    // try for `ps[i]`.
    let mut stack: Vec<(Binding, usize)> = Vec::with_capacity(ps.len() + 1);
    stack.push((b.clone(), 0));
    while !stack.is_empty() {
        let i = stack.len() - 1;
        let (cur, next) = &mut stack[i];
        if i == ps.len() {
            *b = std::mem::take(cur);
            return Some(true);
        }
        let mut found = None;
        while *next < vs.len() {
            let j = *next;
            *next += 1;
            if used[j] {
                continue;
            }
            if !steps.take() {
                return None;
            }
            let mut nb = cur.clone();
            if unify_all(&ps[i], &vs[j], &mut nb, steps)? {
                found = Some((j, nb));
                break;
            }
        }
        match found {
            Some((j, nb)) => {
                used[j] = true;
                stack.push((nb, 0));
            }
            None => {
                // `ps[i]` has no match left: undo the match of `ps[i - 1]` and try its
                // next value statement.
                stack.pop();
                if let Some((_, next)) = stack.last() {
                    used[next - 1] = false;
                }
            }
        }
    }
    Some(false)
}


/// Instantiate a conclusion triple under binding `b` (deeply — variables
/// inside quoted formulae substitute too); `None` if any term stays non-ground.
///
/// A quoted formula is a VALUE: variables remaining inside it after
/// substitution are the formula's OWN quantified variables (e.g. concluding
/// `{ :result :is ?G }` where ?G is bound to a `log:conclusion` closure whose
/// statements include rules — cwm keeps those rule variables quoted in the
/// emitted formula). Only a variable at the triple level (or inside a list)
/// leaves the conclusion genuinely uninstantiated.
fn ground_triple(t: &[Term; 3], b: &Binding, run: &Truncation) -> Option<[Term; 3]> {
    fn instantiated(t: &Term) -> bool {
        match t {
            Term::Var(_) => false,
            Term::List(ms) => ms.iter().all(instantiated),
            Term::Formula(_) => true, // opaque value; inner vars are formula-scoped
            // TRANSPARENT structure (unlike a formula): a variable left inside
            // a concluded quoted triple leaves the conclusion uninstantiated.
            Term::Triple(tr) => tr.iter().all(instantiated),
            _ => true,
        }
    }
    let mut g = [
        bounded::substitute_deep(&t[0], b, run)?,
        bounded::substitute_deep(&t[1], b, run)?,
        bounded::substitute_deep(&t[2], b, run)?,
    ];
    // The conclusion's own `@forAll` universals, now asserted, are document-level ones
    // (GH #6754). After substitution, so no binding can reach them.
    g.iter_mut().for_each(assert_conclusion_universals);
    if g.iter().all(instantiated) {
        Some(g)
    } else {
        None
    }
}

/// Every variable of `stmts` named `from + rest` renamed to `to + rest`, at any depth.
fn rename_var_prefix(stmts: &mut [[Term; 3]], from: &str, to: &str) {
    fn go(t: &mut Term, from: &str, to: &str) {
        match t {
            Term::Var(v) => {
                if let Some(rest) = v.strip_prefix(from) {
                    *t = Term::Var(format!("{to}{rest}"));
                }
            }
            Term::Formula(ts) => ts.iter_mut().flatten().for_each(|m| go(m, from, to)),
            Term::List(ms) => ms.iter_mut().for_each(|m| go(m, from, to)),
            Term::Triple(tr) => tr.iter_mut().for_each(|m| go(m, from, to)),
            _ => {}
        }
    }
    stmts.iter_mut().flatten().for_each(|t| go(t, from, to));
}

/// Rename a conclusion's own universals (`__uc.<iri>`, [`parser::scope_rule`]) to the
/// document-level `__ua.<iri>` — inside a [`formula_closure`] run (`__uk.<k>.<iri>`), to
/// that run's document level, `__ul.<k>.<iri>`, so the result formula gets them back as its
/// own: asserting `∀x. S(x)` beside a document under `∀x` is the
/// same as conjoining `S(x)` under it, since `x` does not occur in what is already there.
fn assert_conclusion_universals(t: &mut Term) {
    match t {
        Term::Var(v) => {
            if let Some(iri) = v.strip_prefix(serialize::CONCLUSION_UNIVERSAL) {
                *t = Term::Var(format!("{}{iri}", serialize::UNIVERSAL_VAR));
            } else if let Some(rest) = v.strip_prefix(CLOSURE_CONCLUSION_UNIVERSAL) {
                *t = Term::Var(format!("{LIFTED_UNIVERSAL}{rest}"));
            }
        }
        Term::Formula(ts) => ts.iter_mut().flatten().for_each(assert_conclusion_universals),
        Term::List(ms) => ms.iter_mut().for_each(assert_conclusion_universals),
        Term::Triple(tr) => tr.iter_mut().for_each(assert_conclusion_universals),
        _ => {}
    }
}

/// What a premise relation reads from the fact store, declared once per registry entry
/// (every builtin, the virtual list relations, and stored predicates). The semi-naive
/// scheduler ([`reads_store_outside_joins`]) and the stratifier
/// ([`strata::stratify`]) derive their store-read facts from this declaration only. Each
/// registry enum declares it in an exhaustive `match` with no wildcard arm, so a new
/// builtin or relation does not compile until it says what it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreRead {
    /// Reads only its operands' values.
    Nothing,
    /// Matched against the stored triples of its predicate, through the join (a new
    /// triple is in some round's delta).
    Joins,
    /// Walks the `rdf:first`/`rdf:rest` cells of one operand in the store, unless that
    /// operand is written as a `( … )` list term.
    ListCells(Operand),
    /// Walks the `rdf:first`/`rdf:rest` cells of every member of its subject list.
    MemberListCells,
    /// Matched against stored triples AND against list terms bound at its subject
    /// (virtual `rdf:first`/`rdf:rest`); the list-term matches are in no delta.
    VirtualList,
    /// Runs a nested closure over its subject (a formula, or a stored list of formulae).
    Nested,
    /// A formula scope over the store (containment, negation, aggregation).
    Scoped,
}

/// Which operand of a premise atom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operand {
    Subject,
    Object,
}

/// The virtual relations: join atoms that can also match list terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VirtualRel {
    /// `rdf:first`.
    First,
    /// `rdf:rest`.
    Rest,
    /// A variable predicate, which may be bound to `rdf:first` or `rdf:rest`.
    AnyPredicate,
}

/// The registry entry a premise predicate resolves to.
#[derive(Clone, Copy, Debug)]
enum Relation {
    Builtin(Builtin),
    Func(Func),
    Binder(Bidi),
    ListGen(ListGen),
    Scope(ScopeOp),
    Collect(CollectOp),
    Virtual(VirtualRel),
    /// Any other predicate: stored triples only.
    Stored,
}

/// Resolve a premise predicate to its registry entry.
fn relation(p: &Term) -> Relation {
    if let Some(op) = scope_op(p) {
        return Relation::Scope(op);
    }
    if let Some(op) = collect_op(p) {
        return Relation::Collect(op);
    }
    if let Some(g) = list_generator(p) {
        return Relation::ListGen(g);
    }
    if let Some(f) = functional_builtin(p) {
        return Relation::Func(f);
    }
    if let Some(op) = binder_builtin(p) {
        return Relation::Binder(op);
    }
    if let Some(op) = builtin(p) {
        return Relation::Builtin(op);
    }
    match p {
        Term::Iri(i) if i == parser::RDF_FIRST => Relation::Virtual(VirtualRel::First),
        Term::Iri(i) if i == parser::RDF_REST => Relation::Virtual(VirtualRel::Rest),
        Term::Var(_) | Term::Blank(_) => Relation::Virtual(VirtualRel::AnyPredicate),
        _ => Relation::Stored,
    }
}

impl Relation {
    fn store_read(self) -> StoreRead {
        match self {
            Relation::Builtin(op) => op.store_read(),
            Relation::Func(f) => f.store_read(),
            Relation::Binder(op) => op.store_read(),
            Relation::ListGen(g) => g.store_read(),
            Relation::Scope(op) => op.store_read(),
            Relation::Collect(op) => op.store_read(),
            Relation::Virtual(v) => v.store_read(),
            Relation::Stored => StoreRead::Joins,
        }
    }
}

impl VirtualRel {
    fn store_read(self) -> StoreRead {
        match self {
            VirtualRel::First | VirtualRel::Rest | VirtualRel::AnyPredicate => {
                StoreRead::VirtualList
            }
        }
    }
}

impl Builtin {
    fn store_read(self) -> StoreRead {
        match self {
            Builtin::Gt
            | Builtin::Lt
            | Builtin::NotGt
            | Builtin::NotLt
            | Builtin::MathEq
            | Builtin::MathNe
            | Builtin::LogEq
            | Builtin::LogNe
            | Builtin::StrContains
            | Builtin::StrStarts
            | Builtin::StrEnds
            | Builtin::StrGt
            | Builtin::StrLt
            | Builtin::StrMatches
            | Builtin::StrNotMatches
            | Builtin::StrContainsIgnCase
            | Builtin::StrNotGt
            | Builtin::StrNotLt
            | Builtin::StrEqIgnCase
            | Builtin::StrNeIgnCase
            | Builtin::StrContainsRoughly => StoreRead::Nothing,
        }
    }
}

impl Bidi {
    fn store_read(self) -> StoreRead {
        match self {
            Bidi::LogUri => StoreRead::Nothing,
        }
    }
}

impl ListGen {
    fn store_read(self) -> StoreRead {
        match self {
            ListGen::Member | ListGen::Iterate => StoreRead::ListCells(Operand::Subject),
            ListGen::In => StoreRead::ListCells(Operand::Object),
        }
    }
}

impl ScopeOp {
    fn store_read(self) -> StoreRead {
        match self {
            ScopeOp::Includes | ScopeOp::NotIncludes | ScopeOp::Supports => StoreRead::Scoped,
        }
    }
}

impl CollectOp {
    fn store_read(self) -> StoreRead {
        match self {
            CollectOp::CollectAll | CollectOp::ForAll => StoreRead::Scoped,
        }
    }
}

impl Func {
    fn store_read(self) -> StoreRead {
        match self {
            // Reads its two literal members before any list walk.
            Func::Dtlit => StoreRead::Nothing,
            Func::Append => StoreRead::MemberListCells,
            Func::LogConclusion => StoreRead::Nested,
            // Every other functional builtin resolves a subject that is not a `( … )`
            // term as a stored data list (or a single value when it is not a list).
            Func::Sum
            | Func::Difference
            | Func::Product
            | Func::Quotient
            | Func::Remainder
            | Func::IntegerQuotient
            | Func::Max
            | Func::Min
            | Func::Exponentiation
            | Func::Logarithm
            | Func::Atan2
            | Func::MemberCount
            | Func::Concat
            | Func::Format
            | Func::Scrape
            | Func::Length
            | Func::StrLength
            | Func::Replace
            | Func::First
            | Func::Last
            | Func::Conjunction
            | Func::ParsedAsN3
            | Func::Langlit
            | Func::Semantics
            | Func::Content
            | Func::LowerCase
            | Func::UpperCase
            | Func::EncodeForUri
            | Func::EncodeForUriCwm
            | Func::EncodeForFragId
            | Func::Negation
            | Func::AbsoluteValue
            | Func::Rounded
            | Func::Floor
            | Func::Ceiling
            | Func::Sin
            | Func::Cos
            | Func::Tan
            | Func::Asin
            | Func::Acos
            | Func::Atan
            | Func::Sinh
            | Func::Cosh
            | Func::Tanh
            | Func::Asinh
            | Func::Acosh
            | Func::Atanh
            | Func::Degrees
            | Func::Radians
            | Func::Year
            | Func::Month
            | Func::Day
            | Func::Hours
            | Func::Minutes
            | Func::Seconds
            | Func::DayOfWeek
            | Func::TimeZone
            | Func::InSeconds => StoreRead::ListCells(Operand::Subject),
        }
    }
}

/// Declares a builtin registry enum and, for tests, `ALL` (every variant), so a test can
/// walk the whole registry.
macro_rules! registry_enum {
    ($(#[$m:meta])* enum $name:ident { $($(#[$vm:meta])* $v:ident),* $(,)? }) => {
        $(#[$m])*
        enum $name { $($(#[$vm])* $v),* }
        #[cfg(test)]
        impl $name {
            const ALL: &'static [$name] = &[$($name::$v),*];
        }
    };
}

registry_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Builtin {
        // numeric (math:)
        Gt,
        Lt,
        NotGt,
        NotLt,
        MathEq,
        MathNe,
        // term (log:)
        LogEq,
        LogNe,
        // string (string:)
        StrContains,
        StrStarts,
        StrEnds,
        StrGt,
        StrLt,
        StrMatches,        // string:matches (regex)
        StrNotMatches,     // string:notMatches (regex, negated; invalid regex ⇒ premise fails)
        StrContainsIgnCase, // string:containsIgnoringCase
        StrNotGt,           // string:notGreaterThan
        StrNotLt,           // string:notLessThan
        StrEqIgnCase,       // string:equalIgnoringCase
        StrNeIgnCase,       // string:notEqualIgnoringCase
        StrContainsRoughly, // string:containsRoughly — case- and whitespace-insensitive
    }
}

fn builtin(p: &Term) -> Option<Builtin> {
    let Term::Iri(i) = p else { return None };
    if let Some(f) = i.strip_prefix(MATH) {
        return Some(match f {
            "greaterThan" => Builtin::Gt,
            "lessThan" => Builtin::Lt,
            "notGreaterThan" => Builtin::NotGt,
            "notLessThan" => Builtin::NotLt,
            "equalTo" => Builtin::MathEq,
            "notEqualTo" => Builtin::MathNe,
            _ => return None,
        });
    }
    if let Some(f) = i.strip_prefix(LOG) {
        return Some(match f {
            "equalTo" => Builtin::LogEq,
            "notEqualTo" => Builtin::LogNe,
            _ => return None,
        });
    }
    if let Some(f) = i.strip_prefix(STRING) {
        return Some(match f {
            "contains" => Builtin::StrContains,
            "containsIgnoringCase" => Builtin::StrContainsIgnCase,
            "containsRoughly" => Builtin::StrContainsRoughly,
            "equalIgnoringCase" => Builtin::StrEqIgnCase,
            "notEqualIgnoringCase" => Builtin::StrNeIgnCase,
            "startsWith" => Builtin::StrStarts,
            "endsWith" => Builtin::StrEnds,
            "greaterThan" => Builtin::StrGt,
            "lessThan" => Builtin::StrLt,
            "notGreaterThan" => Builtin::StrNotGt,
            "notLessThan" => Builtin::StrNotLt,
            "matches" => Builtin::StrMatches,
            "notMatches" => Builtin::StrNotMatches,
            _ => return None,
        });
    }
    None
}

/// Evaluate a filter builtin. Every fallible step inside settles into a local
/// [`bounded::Pending`], so the caller gets a [`Bounded`] it must settle on the run.
fn eval_builtin(op: Builtin, s: &Term, o: &Term, b: &Binding, run: &Truncation) -> Bounded<bool> {
    let pending = bounded::Pending::default();
    let v = eval_builtin_inner(op, s, o, b, run, &pending);
    pending.finish(v)
}

fn eval_builtin_inner(
    op: Builtin,
    s: &Term,
    o: &Term,
    b: &Binding,
    run: &Truncation,
    pending: &bounded::Pending,
) -> bool {
    let (Some(s), Some(o)) = (bounded::substitute(s, b, run), bounded::substitute(o, b, run)) else {
        return false;
    };
    match op {
        Builtin::LogEq => s == o,
        Builtin::LogNe => s != o,
        Builtin::StrContains
        | Builtin::StrStarts
        | Builtin::StrEnds
        | Builtin::StrGt
        | Builtin::StrLt
        | Builtin::StrNotGt
        | Builtin::StrNotLt
        | Builtin::StrMatches
        | Builtin::StrNotMatches
        | Builtin::StrContainsIgnCase
        | Builtin::StrEqIgnCase
        | Builtin::StrNeIgnCase
        | Builtin::StrContainsRoughly => {
            let (Some(x), Some(y)) = (lex(&s), lex(&o)) else { return false };
            match op {
                Builtin::StrContains => x.contains(y),
                Builtin::StrStarts => x.starts_with(y),
                Builtin::StrEnds => x.ends_with(y),
                Builtin::StrGt => x > y,
                Builtin::StrLt => x < y,
                Builtin::StrNotGt => x <= y,
                Builtin::StrNotLt => x >= y,
                Builtin::StrMatches => {
                    settle(pending, bounded::regex(y)).is_some_and(|re| re.is_match(x))
                }
                Builtin::StrNotMatches => {
                    settle(pending, bounded::regex(y)).is_some_and(|re| !re.is_match(x))
                }
                Builtin::StrContainsIgnCase => x.to_lowercase().contains(&y.to_lowercase()),
                Builtin::StrContainsRoughly => {
                    // cwm roughly.n3: case-insensitive, any whitespace run = one space.
                    let norm = |t: &str| {
                        t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
                    };
                    norm(x).contains(&norm(y))
                }
                Builtin::StrEqIgnCase => x.to_lowercase() == y.to_lowercase(),
                Builtin::StrNeIgnCase => x.to_lowercase() != y.to_lowercase(),
                _ => unreachable!(),
            }
        }
        _ => {
            // XPath numeric type promotion by datatype (GH #6745): exact for
            // integer/decimal pairs, IEEE in the float or double tier.
            let (Some(x), Some(y)) = (cmp_num(&s, false), cmp_num(&o, false)) else {
                return false;
            };
            let ord = cmp_promoted(x, y);
            use std::cmp::Ordering::{Equal, Greater, Less};
            match op {
                Builtin::Gt => ord == Some(Greater),
                Builtin::Lt => ord == Some(Less),
                Builtin::NotGt => matches!(ord, Some(Less | Equal)),
                Builtin::NotLt => matches!(ord, Some(Greater | Equal)),
                Builtin::MathEq => ord == Some(Equal),
                Builtin::MathNe => ord != Some(Equal),
                _ => unreachable!(),
            }
        }
    }
}

registry_enum! {
    /// Bidirectional binary builtins over a SINGLE subject term (not a `( … )` list): either
    /// side may be the unknown, and evaluating binds it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Bidi {
        LogUri, // log:uri — IRI ↔ its text as an xsd:string (either direction)
    }
}

fn binder_builtin(p: &Term) -> Option<Bidi> {
    let Term::Iri(i) = p else { return None };
    match i.strip_prefix(LOG) {
        Some("uri") => Some(Bidi::LogUri),
        _ => None,
    }
}

/// Evaluate a bidirectional builtin: compute the bound side, unify with the other (binding
/// a variable or filtering on equality). `None` ⇒ the premise fails for this binding.
/// Evaluate a bidirectional builtin. It has no fallible step today; it returns a
/// [`Bounded`] like every builtin so that any future one must be settled.
fn eval_binder(op: Bidi, s: &Term, o: &Term, b: Binding, run: &Truncation) -> Bounded<Option<Binding>> {
    let pending = bounded::Pending::default();
    let v = eval_binder_inner(op, s, o, b, run, &pending);
    pending.finish(v)
}

fn eval_binder_inner(
    op: Bidi,
    s: &Term,
    o: &Term,
    b: Binding,
    run: &Truncation,
    pending: &bounded::Pending,
) -> Option<Binding> {
    // Both sides as bound, borrowed: the result is sized from them before it is built.
    fn side<'t>(t: &'t Term, b: &'t Binding) -> &'t Term {
        match t {
            Term::Var(v) => b.get(v).unwrap_or(t),
            other => other,
        }
    }
    let (sv, ov) = (side(s, &b), side(o, &b));
    let result = match op {
        Bidi::LogUri => match (sv, ov) {
            // forward: IRI subject → its text as a string literal
            (Term::Iri(i), _) => {
                if uri_refused(run, pending, i, true) {
                    return None;
                }
                (o, Term::Lit(i.clone(), "http://www.w3.org/2001/XMLSchema#string".into(), None))
            }
            // reverse: string object → the IRI it names
            (Term::Var(_), Term::Lit(v, _, _)) => {
                if uri_refused(run, pending, v, false) {
                    return None;
                }
                (s, Term::Iri(v.clone()))
            }
            _ => return None,
        },
    };
    let mut nb = b;
    unify_term(result.0, &result.1, &mut nb, run).then_some(nb)
}

registry_enum! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ListGen {
        Member,  // ?list list:member ?x
        In,      // ?x list:in ?list
        Iterate, // ?list list:iterate (?index ?value) — 0-based, one binding per member
    }
}

fn list_generator(p: &Term) -> Option<ListGen> {
    let Term::Iri(i) = p else { return None };
    match i.strip_prefix(LIST) {
        Some("member") => Some(ListGen::Member),
        Some("in") => Some(ListGen::In),
        Some("iterate") => Some(ListGen::Iterate),
        _ => None,
    }
}

registry_enum! {
    /// The formula-scope operators.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ScopeOp {
        Includes,
        NotIncludes,
        Supports, // includes after closing the scope under its own rules
    }
}

fn scope_op(p: &Term) -> Option<ScopeOp> {
    let Term::Iri(i) = p else { return None };
    match i.strip_prefix(LOG) {
        Some("includes") => Some(ScopeOp::Includes),
        Some("notIncludes") => Some(ScopeOp::NotIncludes),
        Some("supports") => Some(ScopeOp::Supports),
        _ => None,
    }
}

registry_enum! {
    /// The scoped AGGREGATION / universal-quantification operators (EYE and the
    /// N3 builtins spec; they share the scope convention of the [`ScopeOp`]s).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CollectOp {
        /// `( ?template { clause } ?list ) log:collectAllIn ?scope` — findall.
        CollectAll,
        /// `( { clause-a } { clause-b } ) log:forAllIn ?scope` — every solution
        /// of clause-a extends to one of clause-b.
        ForAll,
    }
}

fn collect_op(p: &Term) -> Option<CollectOp> {
    let Term::Iri(i) = p else { return None };
    match i.strip_prefix(LOG) {
        Some("collectAllIn") => Some(CollectOp::CollectAll),
        Some("forAllIn") => Some(CollectOp::ForAll),
        _ => None,
    }
}


/// The prefix of a `log:includes` pattern blank turned wildcard ([`formula_containment`]).
/// Unforgeable (no source variable contains a `.`), and the one variable that may match a
/// scope's own `@forAll` universal: its binding never leaves the containment test.
const PATTERN_WILDCARD: &str = "__w.";

/// SYNTACTIC containment of `pattern` in `scope` (log:includes): every
/// pattern triple must match a scope triple (or be virtual list structure),
/// with pattern blanks as wildcards, pattern variables binding, and scope
/// terms — including its quantified variables — as opaque constants. Returns
/// one binding per complete match.
/// The empty scope.
const NO_STATEMENTS: &[[Term; 3]] = &[];

fn formula_containment<S: std::borrow::Borrow<[Term; 3]>>(
    scope: &[S],
    pattern: &[[Term; 3]],
    seed: &Binding,
    bw: &BwCtx,
) -> Vec<Binding> {
    // An evaluation boundary ([`within_depth`]): a seed or scope past the term-depth
    // limit is not searched.
    if !within_depth(seed, bw)
        || !bw.settle(bounded::generated(&bw.truncated, scope.iter().flat_map(|s| s.borrow().iter())))
    {
        return Vec::new();
    }
    // Pattern existentials (blanks) become wildcard variables.
    let pat: Vec<[Term; 3]> = pattern
        .iter()
        .map(|r| {
            let w = |t: &Term| match t {
                Term::Blank(l) => Term::Var(format!("{PATTERN_WILDCARD}{l}")),
                other => other.clone(),
            };
            [w(&r[0]), w(&r[1]), w(&r[2])]
        })
        .collect();
    let mut out = Vec::new();
    let mut steps = bounded::StepBudget::containment();
    let mut unify = bounded::StepBudget::unification();
    containment_search(&pat, scope, seed.clone(), &mut out, (&mut steps, &mut unify), bw);
    bw.settle(steps.finish(out))
}

/// The depth-first search of [`formula_containment`]: every binding under which each
/// pattern statement matches a scope statement (or virtual list structure), in match
/// order, pushed to `out`. Each step of the search is one visit of a pattern statement,
/// taken from the first budget of `steps`; the unifications it runs share the second
/// (a unification budget), and one that runs out spends the first, which stops the
/// search as a cut. The search state is an explicit stack (one entry per matched
/// pattern statement), so patterns of any width take no stack per statement. Every
/// binding the search extends is checked against the term-depth limit
/// ([`within_depth`]) before it is carried to the next statement, so every binding it
/// carries, clones and returns is within the limit: virtual list access substitutes the
/// binding into the pattern's own list subject, and can build a deeper value than it
/// read.
fn containment_search<S: std::borrow::Borrow<[Term; 3]>>(
    pat: &[[Term; 3]],
    scope: &[S],
    seed: Binding,
    out: &mut Vec<Binding>,
    (steps, unify): (&mut bounded::StepBudget, &mut bounded::StepBudget),
    bw: &BwCtx,
) {
    // A unification that ran out of steps: no match, and the search stops as a cut.
    let unified = |r: Option<bool>, steps: &mut bounded::StepBudget| {
        r.unwrap_or_else(|| {
            steps.spend();
            false
        })
    };
    /// The order the pattern statements are matched in, as indices into `pat`: as
    /// written, until a statement is deferred behind the others.
    #[derive(Clone)]
    enum Order {
        Written(usize),
        Deferred(std::rc::Rc<[usize]>),
    }
    impl Order {
        fn len(&self) -> usize {
            match self {
                Order::Written(n) => *n,
                Order::Deferred(o) => o.len(),
            }
        }
        fn get(&self, at: usize) -> Option<usize> {
            match self {
                Order::Written(n) => (at < *n).then_some(at),
                Order::Deferred(o) => o.get(at).copied(),
            }
        }
        /// The statements after `at`, then the one at `at`.
        fn defer(&self, at: usize) -> Order {
            let order = (at + 1..self.len()).chain([at]).filter_map(|k| self.get(k));
            Order::Deferred(order.collect())
        }
    }
    /// A visit: the pattern statements still to match (`order` from `at`), the binding
    /// so far, and how many more times a statement whose list subject is unbound may be
    /// deferred behind the others.
    struct Visit {
        order: Order,
        at: usize,
        b: Binding,
        defers_left: usize,
    }
    /// Pattern statement `pi` being matched against the scope: its visit, and the next
    /// scope statement to try.
    struct Scan {
        visit: Visit,
        pi: usize,
        next: usize,
    }
    // At most one scan per pattern statement is in progress.
    let mut scans: Vec<Scan> = Vec::with_capacity(pat.len());
    let mut visit = Some(Visit {
        order: Order::Written(pat.len()),
        at: 0,
        b: seed,
        defers_left: pat.len(),
    });
    loop {
        if let Some(Visit {
            order,
            at,
            b,
            defers_left,
        }) = visit.take()
        {
            if !steps.take() {
                return;
            }
            let Some(pi) = order.get(at) else {
                // Each match is a binding row, charged before it is kept
                // ([`bounded::JoinWork`]); a spent allowance ends the search.
                if !bw.work.charge(1, &bw.truncated) {
                    return;
                }
                out.push(b);
                continue;
            };
            let p = &pat[pi];
            let rest = order.len() - at - 1;
            // Virtual rdf:first/rest over a list value — list structure is part of
            // the formula's content for matching purposes (cwm builtins.n3 test2/4).
            if let Term::Iri(pred) = &p[1] {
                if pred == parser::RDF_FIRST || pred == parser::RDF_REST {
                    let Some(subject) = bounded::substitute(&p[0], &b, &bw.truncated) else {
                        continue;
                    };
                    match subject {
                        Term::List(ms) => {
                            if !ms.is_empty() {
                                // The tail is sized before it is built.
                                let val = if pred == parser::RDF_FIRST {
                                    Some(ms[0].clone())
                                } else {
                                    bounded::list_tail(&ms, &bw.truncated)
                                };
                                let mut nb = b.clone();
                                if val.is_some_and(|val| unified(unify_term_in(&p[2], &val, &mut nb, unify), steps))
                                    && within_depth(&nb, bw)
                                {
                                    let (at, defers_left) = (at + 1, rest);
                                    visit = Some(Visit {
                                        order,
                                        at,
                                        b: nb,
                                        defers_left,
                                    });
                                }
                            }
                            continue;
                        }
                        Term::Var(_) if rest > 0 && defers_left > 0 => {
                            // Subject not yet bound — try the other triples first.
                            let order = order.defer(at);
                            let defers_left = defers_left - 1;
                            visit = Some(Visit {
                                order,
                                at: 0,
                                b,
                                defers_left,
                            });
                            continue;
                        }
                        _ => {} // fall through to plain scope matching
                    }
                }
            }
            scans.push(Scan {
                visit: Visit {
                    order,
                    at,
                    b,
                    defers_left,
                },
                pi,
                next: 0,
            });
        }
        // Resume the innermost scan at its next matching scope statement.
        let Some(scan) = scans.last_mut() else {
            return;
        };
        let v = &scan.visit;
        let p = &pat[scan.pi];
        while let Some(st) = scope.get(scan.next) {
            let st: &[Term; 3] = st.borrow();
            scan.next += 1;
            let mut nb = v.b.clone();
            // A match that bound nothing new extends nothing to check.
            if unified(unify_all(&p[..], &st[..], &mut nb, unify), steps)
                && (nb.len() == v.b.len() || within_depth(&nb, bw))
            {
                visit = Some(Visit {
                    order: v.order.clone(),
                    at: v.at + 1,
                    b: nb,
                    defers_left: v.order.len() - v.at - 1,
                });
                break;
            }
        }
        if visit.is_none() {
            scans.pop();
        }
    }
}

/// A parsed document's statements with rules re-encoded into their surface
/// triple form (log:semantics / log:parsedAsN3 result formulae). The document's universals
/// are already the result formula's own: [`bounded::parse_n3_term`] parses its top level as
/// a formula scope, and [`parser::unscope_rule`] gives each rule side back its own.
fn reencode_statements(parsed: parser::Parsed) -> Vec<[Term; 3]> {
    // the rules' formulas MOVE back into statements: no copy of a parsed formula
    let quote = |ts: Vec<[Term; 3]>| -> Term {
        if ts.is_empty() {
            // the empty formula IS the literal true
            Term::Lit("true".into(), parser::XSD_BOOLEAN.into(), None)
        } else {
            Term::Formula(ts)
        }
    };
    let mut ts = parsed.facts;
    for r in parsed.rules.iter().map(parser::unscope_rule) {
        ts.push([quote(r.premise), Term::Iri(parser::LOG_IMPLIES.into()), quote(r.conclusion)]);
    }
    for r in parsed.backward_rules.iter().map(parser::unscope_rule) {
        ts.push([
            quote(r.conclusion),
            Term::Iri(parser::LOG_IMPLIED_BY.into()),
            quote(r.premise),
        ]);
    }
    ts
}

/// A closed formula's own universal while its statements are reasoned over as a document
/// ([`formula_closure`]): `__ul.<k>.<iri>`, unforgeable like every engine-internal name,
/// with `k` the number of closures in progress (this one included), so a nested closure
/// never takes an enclosing closure's universals for its own.
const LIFTED_UNIVERSAL: &str = "__ul.";

/// A forward rule's own conclusion universal inside the [`formula_closure`] run `k`:
/// `__uk.<k>.<iri>`, asserted as that run's `__ul.<k>.<iri>`.
const CLOSURE_CONCLUSION_UNIVERSAL: &str = "__uk.";

/// The forward closure of a quoted formula under its own `=>` rules
/// (log:supports / log:conclusion): the original triples plus everything a
/// fixpoint run over them derives.
/// The bytes of a formula holding `statements` ([`bounded::term_bytes`] of each term,
/// plus one node), read from the borrowed statements.
fn formula_bytes<'a>(statements: impl Iterator<Item = &'a [Term; 3]>) -> usize {
    statements
        .flatten()
        .map(bounded::term_bytes)
        .fold(bounded::TERM_NODE_BYTES, usize::saturating_add)
}

/// `f` over the closure of formula `ts` under its own rules: its statements plus their
/// derivations, each once, borrowed (nothing is copied to make the closure's formula).
fn with_formula_closure<R>(
    ts: &[[Term; 3]],
    bw: &BwCtx,
    f: impl FnOnce(&[&[Term; 3]]) -> R,
) -> R {
    // Import-cycle guard ([`VisitedDocs`]). If this exact formula's closure is already in
    // progress up the stack, re-closing it would recurse forever (the A→A / A→B→A / diamond
    // re-import non-termination path through a LIVE `log:semantics` resolver). Break the cycle
    // by returning the statements UNCLOSED — their own facts are preserved, no derivation is
    // lost that a finite closure would have added, and reasoning terminates. (No resolver, no
    // cycle: with `bw.resolver` `None` the set stays empty and this is a no-op.)
    // The run's stack allowance, shared by every level: a chain of distinct documents
    // (which the import-cycle guard below does not catch) nests closures, each holding
    // its own frames on the stack. Past it the closure is refused, as a cut, and the
    // error fails the enclosing run. `_frames` holds this closure's units until it ends.
    // An evaluation boundary ([`within_depth`]): statements past the term-depth limit
    // are not admitted as a nested run's facts and rules; the formula is left unclosed,
    // as a cut.
    if !bw.settle(bounded::generated(&bw.truncated, ts.iter().flatten())) {
        return f(&ts.iter().collect::<Vec<_>>());
    }
    let _frames = match bounded::enter_closure(&bw.truncated) {
        Ok(level) => level,
        Err(e) => {
            bw.nested_error.borrow_mut().get_or_insert(e);
            return f(&ts.iter().collect::<Vec<_>>());
        }
    };
    let key = formula_key(ts);
    if !bw.visited.borrow_mut().docs.insert(key) {
        bw.settle(Bounded::cut(
            (),
            "a nested closure re-entered a document already being closed and was left unclosed",
        ));
        return f(&ts.iter().collect::<Vec<_>>());
    }

    let mut facts: Vec<[Term; 3]> = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    let mut backward: Vec<Rule> = Vec::new();
    // The formula's own `@forAll` universals quantify its statements, rules included, so
    // the run reads them as that document's universals (GH #6754); the derivations get
    // them back named for their place in the result formula.
    let depth = bw.visited.borrow().docs.len();
    let lift = format!("{LIFTED_UNIVERSAL}{depth}.");
    let asserted = format!("{CLOSURE_CONCLUSION_UNIVERSAL}{depth}.");
    let mut lifted = ts.to_vec();
    parser::own_universals_to(&mut lifted, &lift);
    for row in lifted {
        match (&row[0], &row[1], &row[2]) {
            (Term::Formula(p), Term::Iri(i), Term::Formula(c)) if i == parser::LOG_IMPLIES => {
                let mut r = Rule { premise: p.clone(), conclusion: c.clone() };
                parser::scope_rule(&mut r);
                rename_var_prefix(&mut r.conclusion, serialize::CONCLUSION_UNIVERSAL, &asserted);
                rules.push(r);
            }
            (Term::Formula(c), Term::Iri(i), Term::Formula(p)) if i == parser::LOG_IMPLIED_BY => {
                let mut r = Rule { premise: p.clone(), conclusion: c.clone() };
                parser::scope_rule(&mut r);
                backward.push(r);
            }
            _ => facts.push(row),
        }
    }
    let parsed =
        parser::Parsed { facts, rules, backward_rules: backward, base: bw.base.clone() };
    // Inherit the parent's import-cycle guard ([`VisitedDocs`]) so a `log:semantics` /
    // `log:content` document active up the stack is still recognised when its own closure
    // re-imports it through this nested run.
    // A negation cycle in a nested document is an error unless the top-level run opted in
    // to the legacy single-pass behaviour: an incomplete nested closure would read as
    // evidence of absence to a consumer that negates over it. The builtin cannot return
    // the error, so it is recorded on the context and fails the enclosing run.
    let cycles = match bw.cycles {
        NegationCycles::SinglePass => NegationCycles::SinglePass,
        _ => NegationCycles::Reject,
    };
    let closed = match run_closure(
        parsed,
        bw.resolver,
        Some(bw.visited.clone()),
        &bw.truncated,
        StepMode::None,
        cycles,
        None,
    ) {
        Ok(closed) => closed.read_further(&bw.truncated),
        Err(e) => {
            bw.nested_error
                .borrow_mut()
                .get_or_insert(format!("in a nested closure: {e}"));
            bw.visited.borrow_mut().docs.remove(&key);
            return f(&ts.iter().collect::<Vec<_>>());
        }
    };
    // Original statements (including the rule statements, which cwm keeps in
    // log:conclusion output) plus the derivations, each once. The derivations name the
    // formula's own universals for their place in the result formula again.
    bw.visited.borrow_mut().docs.remove(&key);
    let mut derived: Vec<[Term; 3]> = closed.all.into_iter().collect();
    parser::own_universals_from(&mut derived, &lift);
    let mut seen: FxHashSet<&[Term; 3]> = ts.iter().collect();
    let mut all: Vec<&[Term; 3]> = ts.iter().collect();
    all.extend(derived.iter().filter(|f| seen.insert(f)));
    f(&all)
}

/// The lexical string of a literal term (for `string:` builtins).
fn lex(t: &Term) -> Option<&str> {
    match t {
        Term::Lit(v, _, _) => Some(v.as_str()),
        _ => None,
    }
}

/// RFC 3986 percent-encoding as `string:encodeForUri` computes it: every UTF-8 byte
/// outside the *unreserved* set (`ALPHA / DIGIT / "-" / "." / "_" / "~"`, RFC 3986
/// §2.3) becomes `%XX` with uppercase hex (§2.1's canonical form). This is XPath
/// `fn:encode-for-uri` / SPARQL `ENCODE_FOR_URI`, the strictest of the encoding
/// flavours — reserved delimiters (`/ ? # & = :` …) and spaces are all encoded, so
/// the result is safe to splice into ANY URI component and the encoding is
/// injective (no input can smuggle a delimiter past it).
///
/// Public (not just the builtin's engine room) so callers that must mint the same
/// IRIs OUTSIDE a reasoner run — e.g. `sparq-solid`'s session-side pair-principal
/// minting — share one definition instead of re-implementing it.
///
/// # Examples
///
/// ```
/// use sparq_reason::n3::encode_for_uri;
/// assert_eq!(encode_for_uri("AZaz09-._~"), "AZaz09-._~"); // unreserved: untouched
/// assert_eq!(encode_for_uri("a&b=c"), "a%26b%3Dc");       // delimiters: encoded
/// assert_eq!(encode_for_uri("café"), "caf%C3%A9");        // UTF-8 bytes, uppercase hex
/// ```
pub fn encode_for_uri(s: &str) -> String {
    const UNRESERVED: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(s, UNRESERVED).to_string()
}

/// A `math:` comparison operand placed in the XPath numeric type-promotion tower
/// (`op:numeric-equal` / `op:numeric-less-than` / `op:numeric-greater-than`, XPath
/// F&O §4.2 and Appendix B.1): `xsd:integer` (and its derived types) and `xsd:decimal`
/// keep their exact value, `xsd:float` and `xsd:double` their IEEE value (GH #6745).
#[derive(Clone, Copy, Debug)]
enum CmpNum<'a> {
    /// An integer or decimal, held as its validated `[+-]digits[.digits]` lexical form,
    /// so it compares exactly at any precision (no `i128` or `f64` bound).
    Exact(&'a str),
    /// An `xsd:float`.
    Float(f32),
    /// An `xsd:double` (or a double-shaped untyped lexical).
    Double(f64),
}

impl CmpNum<'_> {
    /// The value promoted to `xsd:double`: one correctly-rounded conversion of an exact
    /// value, an exact widening of a float.
    fn promote_double(self) -> Option<f64> {
        match self {
            CmpNum::Exact(lex) => lex.parse::<f64>().ok(), // no-match: ill-typed (validated in `cmp_num`)
            CmpNum::Float(f) => Some(f64::from(f)),
            CmpNum::Double(d) => Some(d),
        }
    }

    /// The value promoted to `xsd:float` (never reached with a double operand).
    fn promote_float(self) -> Option<f32> {
        match self {
            CmpNum::Exact(lex) => lex.parse::<f32>().ok(), // no-match: ill-typed (validated in `cmp_num`)
            CmpNum::Float(f) => Some(f),
            CmpNum::Double(_) => None,
        }
    }
}

/// The promoted value of a `math:` comparison operand, by its DATATYPE: an integer type
/// or `xsd:decimal` is exact, `xsd:float` / `xsd:double` are IEEE (with the XSD `INF` /
/// `-INF` / `NaN` spellings). A literal of any other datatype (a string) is coerced by its
/// lexical shape, as EYE does: an exponent or `INF` / `NaN` reads as a double, a plain
/// `[+-]digits[.digits]` numeral as an exact number. An ill-typed lexical form, or a
/// non-literal, is not numeric (`None`: the builtin does not match).
///
/// The one exception is `xsd:decimal`'s `inf` / `-inf` / `NaN`: not valid decimals, but
/// the forms `number_term` gives a non-finite f64 fallback result (`(0.5 -1024)
/// math:exponentiation`), so they read as ±INF / NaN (doubles) and the result still
/// matches a bound object or a later comparison.
///
/// Whitespace: the XSD numeric datatypes have `whiteSpace=collapse`, so leading and
/// trailing XML whitespace (space, tab, CR, LF) of a TYPED numeric literal is collapsed
/// before validation (`" 3"^^xsd:integer` is 3; an internal space stays ill-typed). A
/// string operand is trimmed only when `trim_strings` is set — the bound-object value
/// check of a functional builtin (`(1 2) math:sum " 3"` holds); the six comparisons
/// leave it as written, so `" 3"` is not numeric there.
fn cmp_num(t: &Term, trim_strings: bool) -> Option<CmpNum<'_>> {
    use sparq_substrate::numeric::{parse_xsd_f32, parse_xsd_f64, split_decimal};
    let Term::Lit(v, dt, _) = t else { return None };
    // XSD whiteSpace=collapse for the numeric datatypes (leading/trailing XML whitespace).
    let collapsed = v.trim_matches([' ', '\t', '\r', '\n']);
    // An integer type (with its facets, e.g. `xsd:positiveInteger`) or `xsd:decimal`.
    if dt == parser::XSD_DECIMAL {
        // The non-finite forms `number_term` renders for an overflowing f64 fallback
        // (`"inf"` / `"-inf"`, Rust's `NaN`), read as ±INF / NaN as `numval` reads them.
        match collapsed {
            "inf" => return Some(CmpNum::Double(f64::INFINITY)),
            "-inf" => return Some(CmpNum::Double(f64::NEG_INFINITY)),
            "NaN" => return Some(CmpNum::Double(f64::NAN)),
            _ => {}
        }
    }
    if sparq_core::is_integer_datatype(dt) || dt == parser::XSD_DECIMAL {
        // no-match: ill-typed (not of its integer or decimal type)
        return sparq_core::numeric_literal_valid(collapsed, dt)
            .then_some(CmpNum::Exact(collapsed));
    }
    match dt.as_str() {
        parser::XSD_DOUBLE => parse_xsd_f64(collapsed).map(CmpNum::Double), // no-match: ill-typed (not a double)
        "http://www.w3.org/2001/XMLSchema#float" => parse_xsd_f32(collapsed).map(CmpNum::Float), // no-match: ill-typed (not a float)
        _ => {
            // A string is coerced as written, or trimmed as `numval` trims (bound object).
            let v = if trim_strings { v.trim() } else { v.as_str() };
            match v {
                "INF" | "+INF" => Some(CmpNum::Double(f64::INFINITY)),
                "-INF" => Some(CmpNum::Double(f64::NEG_INFINITY)),
                "NaN" => Some(CmpNum::Double(f64::NAN)),
                _ if !v.contains(['e', 'E']) && v.trim() == v && split_decimal(v).is_some() => {
                    Some(CmpNum::Exact(v))
                }
                _ => v.parse::<f64>().ok().map(CmpNum::Double), // no-match: ill-typed (not numeric)
            }
        }
    }
}

/// XPath `op:numeric-*` order of two promoted operands: integer/decimal pairs compare
/// exactly on their digits (any precision); otherwise both are promoted to the least
/// common type — `xsd:float` when neither is a double, else `xsd:double` — and compared
/// as IEEE-754. `None` is unordered (a `NaN` operand), so every comparison but
/// `math:notEqualTo` is false.
fn cmp_promoted(a: CmpNum, b: CmpNum) -> Option<std::cmp::Ordering> {
    use sparq_substrate::numeric::cmp_plain_decimal;
    match (a, b) {
        // not-a-cut: exact-on-digits
        (CmpNum::Exact(x), CmpNum::Exact(y)) => cmp_plain_decimal(x, y),
        // not-a-cut: defined-float (XPath promotes both operands to xsd:double)
        (CmpNum::Double(_), _) | (_, CmpNum::Double(_)) => {
            a.promote_double()?.partial_cmp(&b.promote_double()?)
        }
        // not-a-cut: defined-float (XPath promotes both operands to xsd:float)
        _ => a.promote_float()?.partial_cmp(&b.promote_float()?),
    }
}

/// EXACT numeric tower for the arithmetic `math:` builtins, matching EYE's
/// Prolog rational arithmetic on the cases the suites exercise: integers and
/// decimals compute exactly (scaled i128), doubles (an `e` exponent or
/// INF/NaN) in f64. Strings coerce by lexical shape — `("2.7" "2")
/// math:difference 0.7` must hold EXACTLY (f64 gives 0.700…0002).
///
/// # Seam-2 tower adoption (sq-pbz04.5.1, `research/rif-core-conformance-and-builtins.md` §2)
///
/// The exact add / subtract / multiply / negate / abs arithmetic and the
/// scale-alignment used by the comparison / max-min paths are now DELEGATED to
/// the SHARED `sparq_substrate::numeric::Dec` (the same exact fixed-point
/// `mant * 10^-scale` decimal the SPARQL engine's FILTER / BIND path uses), so
/// the N3 chainer and the engine can never diverge on exact-decimal arithmetic.
/// This enum stays the thin **EYE-compat adapter** over that shared core: the
/// three tiers are exactly EYE's (`Int` in `i128`, exact `Dec`, `f64`) and every
/// EYE-specific edge is preserved byte-for-byte — lexical-shape string coercion
/// ([`numval`]), the `numval_term` result rendering (whole-`f64` → `xsd:integer`,
/// `Dec` normalised via `dec_norm`), `math:remainder`'s divisor-sign integer
/// semantics, `math:integerQuotient`'s aligned-mantissa floor-division, the
/// `math:quotient` scale-34 exactness rule (non-terminating → `f64`, integer /
/// integer exact → `xsd:integer`), integer `math:exponentiation`, and the
/// `Int`-collapse rendering of `floor` / `ceiling`.
///
/// The **i128 ↔ i64 wrinkle** the seam names: the chainer's `Int` tier is `i128`
/// while substrate `Num::Int` is `i64`, so this adapter never carries a substrate
/// `Num::Int` — it converts each exact tier to a substrate `Dec` for the delegated
/// arithmetic (`Int(i)` → `Dec { mant: i, scale: 0 }`, exact for the full `i128`
/// range including `> i64::MAX`; overflow of the `Dec` mantissa falls back to
/// `f64` exactly as the pre-adoption `checked_*` path did). The substrate ops
/// used ([`sparq_substrate::numeric::Dec::checked_add`] / `checked_sub` /
/// `checked_mul` / `cmp`) are the SAME `i128` mantissa operations the private
/// tower had, so the closure is byte-identical (verified by the direct
/// old-vs-substrate differential in the tests below and the unchanged N3/EYE
/// differential + expressivity floors). The EYE-specific ops that the substrate
/// tower deliberately does NOT provide (quotient's scale-34 / type rule,
/// remainder, integer-quotient, exponentiation, floor/ceiling/rounded rendering)
/// keep their EYE algorithm here — reasoned non-adoption of those edges, so the
/// refactor stays behaviour-neutral rather than smuggling a behaviour change
/// through a shared method whose contract differs (e.g. `Dec::checked_div` rounds
/// at scale 18 and always yields a decimal, which EYE's `math:quotient` does not).
#[derive(Clone, Copy, Debug)]
enum NumVal {
    Int(i128),
    /// mantissa, scale: value = mantissa / 10^scale.
    Dec(i128, u32),
    F64(f64),
}

/// EYE-compat bridge to the shared exact-decimal core. The exact tiers (`Int` /
/// `Dec`) map to a substrate [`sparq_substrate::numeric::Dec`]; `F64` has no exact
/// image (returns `None`, so the caller stays on the `f64` fallback path exactly
/// as the private tower did). `Int(i128)` maps to `Dec { mant: i, scale: 0 }` for
/// the FULL `i128` range — this is the i128↔i64 wrinkle: substrate `Num::Int` is
/// `i64`, so an out-of-`i64`-range chainer integer is carried as a scale-0 `Dec`
/// (exact). [OPUS-4.8] sq-pbz04.5.1
#[inline]
fn numval_to_subdec(v: NumVal) -> Option<sparq_substrate::numeric::Dec> {
    match v {
        NumVal::Int(i) => Some(sparq_substrate::numeric::Dec { mant: i, scale: 0 }),
        NumVal::Dec(m, s) => Some(sparq_substrate::numeric::Dec { mant: m, scale: s }),
        NumVal::F64(_) => None,
    }
}

/// EYE `math:negation`, tier-preserving. The exact tiers negate the substrate
/// `Dec` mantissa via `i128::checked_neg` (so `i128::MIN` overflows → `None`, the
/// premise fails, exactly as the private tower did — the substrate `Num::neg`
/// would instead fall back to `Double`, a NON-neutral edge, so negation keeps its
/// EYE None-on-overflow semantics here). [OPUS-4.8] sq-pbz04.5.1
#[inline]
fn numval_negate(v: NumVal) -> Option<NumVal> {
    match v {
        NumVal::Int(i) => Some(NumVal::Int(i.checked_neg()?)), // not-a-cut: settled-by-caller
        NumVal::Dec(m, s) => Some(NumVal::Dec(m.checked_neg()?, s)), // not-a-cut: settled-by-caller
        NumVal::F64(x) => Some(NumVal::F64(-x)),
    }
}

/// EYE `math:absoluteValue`, tier-preserving (`i128::checked_abs`, so `i128::MIN`
/// overflows → `None`; symmetric to [`numval_negate`]). [OPUS-4.8] sq-pbz04.5.1
#[inline]
fn numval_abs(v: NumVal) -> Option<NumVal> {
    match v {
        NumVal::Int(i) => Some(NumVal::Int(i.checked_abs()?)), // not-a-cut: settled-by-caller
        NumVal::Dec(m, s) => Some(NumVal::Dec(m.checked_abs()?, s)), // not-a-cut: settled-by-caller
        NumVal::F64(x) => Some(NumVal::F64(x.abs())),
    }
}

fn numval(t: &Term) -> Option<NumVal> {
    let Term::Lit(v, _, _) = t else { return None };
    let v = v.trim();
    match v {
        "INF" | "+INF" => return Some(NumVal::F64(f64::INFINITY)),
        "-INF" => return Some(NumVal::F64(f64::NEG_INFINITY)),
        "NaN" => return Some(NumVal::F64(f64::NAN)),
        _ => {}
    }
    if v.contains(['e', 'E']) {
        return v.parse::<f64>().ok().map(NumVal::F64); // no-match: ill-typed (not a double lexical form)
    }
    if let Some((int, frac)) = v.split_once('.') {
        // not-a-cut: settled-by-caller (past i128: f64 image; `numval_in` records the cut)
        if let Some(m) = joined_i128(int, frac) {
            return Some(NumVal::Dec(m, frac.len() as u32));
        }
        return v.parse::<f64>().ok().map(NumVal::F64); // no-match: ill-typed (not a decimal lexical form)
    }
    v.parse::<i128>()
        .ok() // not-a-cut: settled-by-caller (`numval_in` records the cut)
        .map(NumVal::Int)
        .or_else(|| v.parse::<f64>().ok().map(NumVal::F64)) // no-match: ill-typed (not numeric)
}

/// `int` and `frac` read as one `i128` numeral, as `format!("{int}{frac}").parse::<i128>()`
/// reads it, without building the joined text: an optional leading sign, then at least one
/// ASCII digit. `None` for any other text, or a value past `i128`.
fn joined_i128(int: &str, frac: &str) -> Option<i128> {
    let mut bytes = int.bytes().chain(frac.bytes()).peekable();
    let neg = match bytes.peek() {
        Some(b'-') => true,
        Some(b'+') => false,
        _ => return digits_i128(bytes, false),
    };
    bytes.next();
    digits_i128(bytes, neg)
}

/// The `i128` the ASCII digits `bytes` spell, negated with `neg` (accumulated toward the
/// sign, so `i128::MIN` reads); `None` for no digits, a non-digit, or a value past `i128`.
fn digits_i128(bytes: impl Iterator<Item = u8>, neg: bool) -> Option<i128> {
    let mut acc: Option<i128> = None;
    for b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        let d = i128::from(b - b'0');
        // not-a-cut: settled-by-caller (past i128: f64 image; `numval_in` records the cut)
        let up = acc.unwrap_or(0).checked_mul(10)?;
        // not-a-cut: settled-by-caller (past i128: f64 image; `numval_in` records the cut)
        acc = Some(if neg { up.checked_sub(d)? } else { up.checked_add(d)? });
    }
    acc
}

/// [`numval`] inside a builtin: an integer or decimal numeral whose exact value is past
/// the `i128` tower is read as its `f64` image, which is a cut (the exact value exists
/// but cannot be represented).
fn numval_in(t: &Term, pending: &bounded::Pending) -> Option<NumVal> {
    let v = numval(t)?;
    if let (NumVal::F64(_), Term::Lit(lex, _, _)) = (v, t) {
        settle(pending, bounded::exact_numeral(lex.trim()));
    }
    Some(v)
}

/// A number used as a whole `i64` exactly (`time:inSeconds` reverse mode): an integer,
/// a decimal with a zero fraction, or a whole double. A fraction or a value past `i64`
/// would lose precision, which is a cut ([`bounded::whole_i64`]).
fn numval_whole_i64(v: NumVal, pending: &bounded::Pending) -> Option<i64> {
    let whole = match v {
        NumVal::Int(i) => Some(i),
        NumVal::Dec(m, sc) => {
            let p = rep(pending, 10i128.checked_pow(sc))?;
            (m % p == 0).then(|| m / p)
        }
        NumVal::F64(f) => return settle(pending, bounded::whole_i64_of_f64(f)),
    };
    settle(pending, bounded::whole_i64(whole))
}

/// `string:format`'s `%f`: the value with 6 fraction digits, as C printf renders it.
/// An integer or decimal is rendered exactly from its digits (a longer fraction rounds
/// half to even); a double is the double's own value.
fn fixed6(v: NumVal, pending: &bounded::Pending) -> Option<String> {
    let (m, sc) = match v {
        NumVal::Int(i) => (i, 0),
        NumVal::Dec(m, sc) => (m, sc),
        // The double's own value (a numeral past i128 was a cut in `numval_in`).
        NumVal::F64(f) => return Some(format!("{f:.6}")),
    };
    if sc > 6 {
        let p = rep(pending, 10i128.checked_pow(sc - 6))?;
        let (q, r) = (m / p, m % p);
        let twice = r.unsigned_abs() * 2;
        let up = twice > p.unsigned_abs() || (twice == p.unsigned_abs() && q % 2 != 0);
        let q = if up { q + m.signum() } else { q };
        // The sign of a value that rounds to zero survives, as in printf (`-0.000000`).
        return Some(fixed_digits(m < 0, q.unsigned_abs(), 6));
    }
    let digits = fixed_digits(m < 0, m.unsigned_abs(), sc);
    Some(format!("{digits}{}", "0".repeat((6 - sc) as usize)))
}

/// `mag × 10^-scale` as `[-]int.frac` with exactly `scale` fraction digits (no point when
/// `scale` is 0).
fn fixed_digits(neg: bool, mag: u128, scale: u32) -> String {
    let scale = scale as usize;
    let mut d = mag.to_string();
    if d.len() <= scale {
        d = format!("{}{d}", "0".repeat(scale + 1 - d.len()));
    }
    let (int, frac) = d.split_at(d.len() - scale);
    let sign = if neg { "-" } else { "" };
    if scale == 0 {
        format!("{sign}{int}.")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

/// The canonical value string of an integer or decimal numeral (`[+-]digits[.digits]`),
/// computed on the digits, so it is exact past `i128` too: leading zeros and a `+` sign
/// dropped, trailing fraction zeros dropped, a whole value written as an integer — the
/// same string the exact tower renders for a value it can hold. `None` for any other
/// lexical form.
pub(crate) fn big_numeral_canonical(lex: &str) -> Option<String> {
    let (sign, int, frac) = big_numeral_parts(lex)?;
    Some(if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    })
}

/// The length of [`big_numeral_canonical`]'s string, computed without building it.
fn big_numeral_canonical_len(lex: &str) -> Option<usize> {
    let (sign, int, frac) = big_numeral_parts(lex)?;
    let frac = if frac.is_empty() { 0 } else { frac.len() + 1 };
    Some(sign.len() + int.len() + frac)
}

/// The sign, integer digits and fraction digits of [`big_numeral_canonical`]'s string,
/// borrowed from `lex`.
fn big_numeral_parts(lex: &str) -> Option<(&'static str, &str, &str)> {
    let lex = lex.trim();
    let (neg, body) = match lex.as_bytes().first()? {
        b'-' => (true, &lex[1..]),
        b'+' => (false, &lex[1..]),
        _ => (false, lex),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    if int.is_empty() && frac.is_empty()
        || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let int = int.trim_start_matches('0');
    let frac = frac.trim_end_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let zero = int == "0" && frac.is_empty();
    let sign = if neg && !zero { "-" } else { "" };
    Some((sign, int, frac))
}

impl NumVal {
    fn to_f64(self) -> f64 {
        match self {
            // The f64 image, for builtins defined over f64; exact callers use the tower.
            // not-a-cut: defined-float
            NumVal::Int(i) => i as f64,
            // not-a-cut: defined-float
            NumVal::Dec(m, s) => m as f64 / 10f64.powi(s as i32),
            NumVal::F64(f) => f,
        }
    }
    /// Both values as scale-aligned (mantissa, scale), when exact.
    fn aligned(a: NumVal, b: NumVal) -> Option<(i128, i128, u32)> {
        let part = |v: NumVal| match v {
            NumVal::Int(i) => Some((i, 0u32)),
            NumVal::Dec(m, s) => Some((m, s)),
            NumVal::F64(_) => None,
        };
        let ((ma, sa), (mb, sb)) = (part(a)?, part(b)?);
        let s = sa.max(sb);
        let up = |m: i128, from: u32| -> Option<i128> {
            m.checked_mul(10i128.checked_pow(s - from)?) // not-a-cut: settled-by-caller
        };
        Some((up(ma, sa)?, up(mb, sb)?, s))
    }
}

/// Drop trailing zero scale: Dec(700, 2) → Dec(7, 1); Dec(30, 1) → Dec(3, 0)
/// stays a Dec (decimal-typed inputs keep the decimal type, lexical `x.0`).
fn dec_norm(m: i128, s: u32) -> (i128, u32) {
    if m == 0 {
        // Every fraction zero drops (without one loop step per zero of a long scale).
        return (0, 0);
    }
    let (mut m, mut s) = (m, s);
    while s > 0 && m % 10 == 0 {
        m /= 10;
        s -= 1;
    }
    (m, s)
}

/// Render a NumVal as an N3 literal, EYE-style: Int → xsd:integer; Dec →
/// xsd:decimal with at least one fraction digit; F64 → integer when whole
/// (matching the old behavior), else decimal lexical.
fn numval_term(v: NumVal) -> Term {
    match v {
        NumVal::Int(i) => {
            Term::Lit(i.to_string(), "http://www.w3.org/2001/XMLSchema#integer".into(), None)
        }
        NumVal::Dec(m, s) => {
            let (m, s) = dec_norm(m, s);
            let neg = m < 0;
            let digits = m.unsigned_abs().to_string();
            let s = s as usize;
            let (int, frac) = if digits.len() > s {
                (digits[..digits.len() - s].to_string(), digits[digits.len() - s..].to_string())
            } else {
                ("0".to_string(), format!("{:0>width$}", digits, width = s))
            };
            let frac = if frac.is_empty() { "0".to_string() } else { frac };
            Term::Lit(
                format!("{}{int}.{frac}", if neg { "-" } else { "" }),
                "http://www.w3.org/2001/XMLSchema#decimal".into(),
                None,
            )
        }
        NumVal::F64(f) => number_term(f),
    }
}

/// A numeric builtin result before its literal is rendered: a value [`numval_term`]
/// renders, or an `f64` [`double_term`] renders. Its size comes from the value (digit
/// count, scale, sign, exponent), so the result is checked against `max_term_bytes`
/// before its lexical form is built (GH #6901).
#[derive(Clone, Copy, Debug)]
enum Numeral {
    Num(NumVal),
    Double(f64),
}

impl Numeral {
    /// A count (a list or string length) as an `xsd:integer`.
    fn count(n: usize) -> Numeral {
        Numeral::Num(NumVal::Int(n as i128))
    }

    /// The datatype IRI of the rendered literal.
    fn datatype(self) -> &'static str {
        match self {
            Numeral::Num(NumVal::Int(_)) => parser::XSD_INTEGER,
            Numeral::Num(NumVal::Dec(..)) => parser::XSD_DECIMAL,
            Numeral::Num(NumVal::F64(f)) if number_is_whole(f) => parser::XSD_INTEGER,
            Numeral::Num(NumVal::F64(_)) => parser::XSD_DECIMAL,
            Numeral::Double(_) => parser::XSD_DOUBLE,
        }
    }

    /// The length of the rendered lexical form, computed without rendering it.
    fn lex_len(self) -> usize {
        match self {
            Numeral::Num(NumVal::Int(i)) => int_lex_len(i),
            Numeral::Num(NumVal::Dec(m, s)) => dec_lex_len(m, s),
            Numeral::Num(NumVal::F64(f)) => number_lex_len(f),
            Numeral::Double(f) => double_lex_len(f),
        }
    }

    /// A cheap upper bound on [`Numeral::lex_len`]: exact for an integer; for a decimal,
    /// the length before trailing fraction zeros are dropped; for an `f64`, the most any
    /// `f64` prints as ([`NUMERAL_TEXT_BYTES`]).
    fn lex_bound(self) -> usize {
        match self {
            Numeral::Num(NumVal::Int(i)) => int_lex_len(i),
            Numeral::Num(NumVal::Dec(m, s)) => digit_count(m.unsigned_abs())
                .max((s as usize).saturating_add(1))
                .saturating_add(3),
            Numeral::Num(NumVal::F64(_)) | Numeral::Double(_) => NUMERAL_TEXT_BYTES,
        }
    }

    /// The [`bounded::term_bytes`] of [`Numeral::term`], computed without building it.
    fn term_bytes(self) -> usize {
        bounded::TERM_NODE_BYTES
            .saturating_add(self.lex_len())
            .saturating_add(self.datatype().len())
    }

    /// A cheap upper bound on [`Numeral::term_bytes`] (every numeric datatype IRI is at
    /// most as long as `xsd:decimal`'s).
    fn term_bound(self) -> usize {
        bounded::TERM_NODE_BYTES
            .saturating_add(self.lex_bound())
            .saturating_add(parser::XSD_DECIMAL.len())
    }

    /// The literal.
    fn term(self) -> Term {
        match self {
            Numeral::Num(v) => numval_term(v),
            Numeral::Double(f) => double_term(f),
        }
    }
}

/// A numeric result of builtin `f`, sized from its value against the run's
/// `max_term_bytes` before its literal is rendered (GH #6901): `None` when it is refused
/// (a cut, and the run's refusal).
fn numeric_result(run: &Truncation, cuts: &impl bounded::Sink, f: Func, n: Numeral) -> Option<Term> {
    (!refused_term(run, cuts, Derived::Result(f), n.term_bound(), || n.term_bytes())).then(|| n.term())
}

/// The number of decimal digits of `u` (`0` has one).
fn digit_count(u: u128) -> usize {
    if u == 0 {
        1
    } else {
        u.ilog10() as usize + 1
    }
}

/// The length of `i.to_string()`.
fn int_lex_len(i: i128) -> usize {
    usize::from(i < 0) + digit_count(i.unsigned_abs())
}

/// The length of [`numval_term`]'s `xsd:decimal` lexical form of `m / 10^s`: trailing
/// fraction zeros dropped, at least one fraction digit, and `0` for an empty integer part.
fn dec_lex_len(m: i128, s: u32) -> usize {
    let (m, s) = dec_norm(m, s);
    let (digits, s) = (digit_count(m.unsigned_abs()), s as usize);
    let body = if digits > s {
        // `int.frac`, or `int.0` when there is no fraction.
        (digits - s).saturating_add(1).saturating_add(s.max(1))
    } else {
        // `0.` then the digits, zero-padded to `s`.
        s.saturating_add(2)
    };
    usize::from(m < 0).saturating_add(body)
}

/// Whether [`number_term`] renders `v` as an `xsd:integer` (whole, below 2^53).
fn number_is_whole(v: f64) -> bool {
    v.fract() == 0.0 && v.abs() < 9.007e15
}

/// The length of [`number_term`]'s lexical form of `v`.
fn number_lex_len(v: f64) -> usize {
    if number_is_whole(v) {
        // not-a-cut: exact-cast (whole, below 2^53)
        lex_len_of(format_args!("{}", v as i64)).len
    } else {
        lex_len_of(format_args!("{v}")).len
    }
}

/// The length of [`double_term`]'s lexical form of `v`.
fn double_lex_len(v: f64) -> usize {
    if v.is_nan() || v == f64::INFINITY {
        3
    } else if v == f64::NEG_INFINITY {
        4
    } else {
        // `{:e}`, with `.0` added to a mantissa that has no fraction digit.
        let n = lex_len_of(format_args!("{v:e}"));
        if n.dot {
            n.len
        } else {
            n.len + 2
        }
    }
}

/// Counts the bytes a formatter writes, without keeping them, and whether one was `.`.
#[derive(Default)]
struct LexLen {
    len: usize,
    dot: bool,
}

impl std::fmt::Write for LexLen {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.len = self.len.saturating_add(s.len());
        self.dot |= s.contains('.');
        Ok(())
    }
}

/// What `args` formats to, counted ([`LexLen`]).
fn lex_len_of(args: std::fmt::Arguments<'_>) -> LexLen {
    let mut n = LexLen::default();
    // Writing to a counter cannot fail.
    let _ = std::fmt::Write::write_fmt(&mut n, args);
    n
}

registry_enum! {
    /// Functional `math:`/`string:`/`list:`/`time:` builtins: the subject is a `( … )` list (or a
    /// single value for the unary ops), and the object is computed.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Func {
        // list-arg
        Sum,
        Difference,
        Product,
        Quotient,
        Remainder,       // math:remainder (a mod b)
        IntegerQuotient, // math:integerQuotient (floor(a/b))
        Max,
        Min,
        Exponentiation,
        Logarithm,   // (x base) math:logarithm log_base(x) — EYE: log(U)/log(V)
        Atan2,       // (x y) math:atan2 — EYE's eye.pl computes atan(x/y), NOT C atan2; we match
        MemberCount, // math:memberCount — list length, or distinct triple count of a formula
        Concat,      // string:concatenation
        Format,      // string:format — ( fmt args… ); %s/%d/%f/%% subset, else premise fails
        Scrape,      // string:scrape — ( str regex ); the FIRST capture group of the first match
        Length,      // list:length
        StrLength,   // string:length (Unicode scalar count)
        Replace,     // string:replace (regex): ( str pattern replacement ) string:replace ?out
        First,       // list:first
        Last,        // list:last
        Append,      // list:append — ( list… ) list:append ?out (first-class list result)
        Conjunction, // log:conjunction — merge a list of formulae into one formula
        Dtlit,       // log:dtlit — ( "lex" xsd:dt ) ↔ "lex"^^xsd:dt (both directions)
        LogConclusion, // log:conclusion — a formula's forward closure, as a formula
        ParsedAsN3,    // log:parsedAsN3 — an N3 source string, parsed to a formula
        Langlit,       // log:langlit — ( "lex" "lang" ) → "lex"@lang
        Semantics,     // log:semantics — a document IRI's parsed formula (needs a Resolver)
        Content,       // log:content — a document IRI's source text (needs a Resolver)
        // single-value-arg (string case mapping, Unicode-aware)
        LowerCase,    // string:lowerCase
        UpperCase,    // string:upperCase
        EncodeForUri, // string:encodeForUri — RFC 3986 percent-encoding (see [`encode_for_uri`])
        EncodeForUriCwm, // string:encodeForURI — cwm's URI quoting (keeps #'()~, encodes /)
        EncodeForFragId, // string:encodeForFragID — cwm's fragment quoting (keeps /, encodes #'()~)
        // single-value-arg (unary math)
        Negation,
        AbsoluteValue,
        Rounded,
        Floor,
        Ceiling,
        // single-value-arg trig/hyperbolic (forward direction only; see module doc)
        Sin,
        Cos,
        Tan,
        Asin,
        Acos,
        Atan,
        Sinh,
        Cosh,
        Tanh,
        Asinh,
        Acosh,
        Atanh,
        Degrees, // radians → degrees (x·180/π), matching eye.pl
        Radians, // degrees → radians (x·π/180)
        // single-value-arg (time: components of an xsd:dateTime)
        Year,
        Month,
        Day,
        Hours,
        Minutes,
        Seconds,
        DayOfWeek, // time:dayOfWeek — 0=Sunday … 6=Saturday (cwm)
        TimeZone,  // time:timeZone — the explicit ±hh:mm offset (absent for Z/none)
        InSeconds, // time:inSeconds — epoch seconds (bidirectional, cwm t1)
    }
}

fn functional_builtin(p: &Term) -> Option<Func> {
    let Term::Iri(i) = p else { return None };
    if let Some(f) = i.strip_prefix(MATH) {
        return Some(match f {
            "sum" => Func::Sum,
            "difference" => Func::Difference,
            "product" => Func::Product,
            "quotient" => Func::Quotient,
            "remainder" => Func::Remainder,
            "integerQuotient" => Func::IntegerQuotient,
            "max" => Func::Max,
            "min" => Func::Min,
            "exponentiation" => Func::Exponentiation,
            "logarithm" => Func::Logarithm,
            "atan2" => Func::Atan2,
            "memberCount" => Func::MemberCount,
            "negation" => Func::Negation,
            "absoluteValue" => Func::AbsoluteValue,
            "rounded" => Func::Rounded,
            "floor" => Func::Floor,
            "ceiling" => Func::Ceiling,
            "sin" => Func::Sin,
            "cos" => Func::Cos,
            "tan" => Func::Tan,
            "asin" => Func::Asin,
            "acos" => Func::Acos,
            "atan" => Func::Atan,
            "sinh" => Func::Sinh,
            "cosh" => Func::Cosh,
            "tanh" => Func::Tanh,
            "asinh" => Func::Asinh,
            "acosh" => Func::Acosh,
            "atanh" => Func::Atanh,
            "degrees" => Func::Degrees,
            "radians" => Func::Radians,
            _ => return None,
        });
    }
    if let Some(f) = i.strip_prefix(TIME) {
        return Some(match f {
            "year" => Func::Year,
            "month" => Func::Month,
            "day" => Func::Day,
            // cwm uses the singular forms; EYE the plural — accept both.
            "hour" | "hours" => Func::Hours,
            "minute" | "minutes" => Func::Minutes,
            "second" | "seconds" => Func::Seconds,
            "dayOfWeek" => Func::DayOfWeek,
            "timeZone" => Func::TimeZone,
            "inSeconds" => Func::InSeconds,
            _ => return None,
        });
    }
    if let Some(f) = i.strip_prefix(LOG) {
        return match f {
            "conjunction" => Some(Func::Conjunction),
            "dtlit" => Some(Func::Dtlit),
            "conclusion" => Some(Func::LogConclusion),
            "parsedAsN3" => Some(Func::ParsedAsN3),
            "langlit" => Some(Func::Langlit),
            "semantics" => Some(Func::Semantics),
            "content" => Some(Func::Content),
            _ => None,
        };
    }
    match (i.strip_prefix(STRING), i.strip_prefix(LIST)) {
        (Some("concatenation"), _) => Some(Func::Concat),
        (Some("format"), _) => Some(Func::Format),
        (Some("scrape"), _) => Some(Func::Scrape),
        (Some("length"), _) => Some(Func::StrLength),
        (Some("replace"), _) => Some(Func::Replace),
        (Some("lowerCase"), _) => Some(Func::LowerCase),
        (Some("upperCase"), _) => Some(Func::UpperCase),
        (Some("encodeForUri"), _) => Some(Func::EncodeForUri),
        (Some("encodeForURI"), _) => Some(Func::EncodeForUriCwm),
        (Some("encodeForFragID"), _) => Some(Func::EncodeForFragId),
        (_, Some("length")) => Some(Func::Length),
        (_, Some("first")) => Some(Func::First),
        (_, Some("last")) => Some(Func::Last),
        (_, Some("append")) => Some(Func::Append),
        _ => None,
    }
}

/// Text a numeric argument may print as in `string:concatenation` / `string:format`, past
/// its lexical form's length: an `f64` in plain decimal notation takes at most about 330
/// bytes (`1e308`, or the smallest subnormal), an `i128` 40.
const NUMERAL_TEXT_BYTES: usize = 400;

/// The datatype of a language-tagged literal.
const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

/// The formula a builtin parses N3 text into (`log:parsedAsN3`, `log:semantics`), sized
/// before it is kept: `None` when the text is not N3 (a no-match; a parser limit is a
/// cut) or when the formula is past the run's term-size limit (a cut, and the run
/// refused). The parser charges what it keeps against the limit before allocating it
/// ([`bounded::parse_n3_term`]), so an expanding parse stops early. Once re-encoded, the
/// formula is sized again at the size it will have with its blanks standardized apart
/// (the parsed statements plus the growth of every blank label), before the run's
/// runtime-document table ([`BwCtx::runtime_formula`]) rewrites any label.
fn parsed_formula(
    f: Func,
    src: &str,
    base: &str,
    bw: &BwCtx,
    pending: &bounded::Pending,
) -> Option<Term> {
    let parse = || {
        // no-match: ill-typed (not N3 text; `parse_n3_term` already recorded a parser limit
        // or the term-size refusal as a cut)
        let parsed = bounded::parse_n3_term(src, base, &bw.truncated, pending)?;
        Some(reencode_statements(parsed))
    };
    let admit = |bytes| !refused_term(&bw.truncated, pending, Derived::Result(f), bytes, || bytes);
    let statements = bw.runtime_formula(src, base, parse, admit, pending)?;
    Some(Term::Formula(statements))
}

/// What a term-size refusal names ([`bounded::derived_term`]).
enum Derived {
    /// The result of a functional builtin.
    Result(Func),
    /// The argument list a functional builtin is evaluated over.
    Arguments(Func),
    /// The list `log:collectAllIn` collects.
    Collected,
    /// The term a list generator binds (`list:member`, `list:in`, `list:iterate`).
    Generated(ListGen),
    /// The term `log:uri` binds.
    Uri,
}

impl std::fmt::Display for Derived {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Derived::Result(b) => write!(f, "the result of the builtin {b:?}"),
            Derived::Arguments(b) => write!(f, "the argument list of the builtin {b:?}"),
            Derived::Collected => write!(f, "the list log:collectAllIn collects"),
            Derived::Generated(g) => write!(f, "the result of the builtin {g:?}"),
            Derived::Uri => write!(f, "the result of the builtin LogUri"),
        }
    }
}

/// The ONE term-size checkpoint of the text engine (GH #6771), over
/// [`bounded::derived_term`]: `true` when the term is refused, its cut recorded on `cuts`
/// and the run refused, which fails every closure of the run ([`Truncation::refusal`]).
fn refused_term(
    run: &Truncation,
    cuts: &impl bounded::Sink,
    what: Derived,
    bound: usize,
    exact: impl FnOnce() -> usize,
) -> bool {
    settle(cuts, bounded::derived_term(run, &what, bound, exact))
}

/// Whether the term `log:uri` binds, built from the borrowed `text`, is refused for its
/// size: with `to_literal`, an IRI's text as an `xsd:string` literal (forward); without,
/// the IRI a string names (reverse). Shared by the text engine, the compiled evaluator
/// and the incremental counting engine.
pub(crate) fn uri_refused(
    run: &Truncation,
    cuts: &impl bounded::Sink,
    text: &str,
    to_literal: bool,
) -> bool {
    let bytes = if to_literal {
        bounded::string_term_bytes(text.len())
    } else {
        bounded::TERM_NODE_BYTES.saturating_add(text.len())
    };
    refused_term(run, cuts, Derived::Uri, bytes, || bytes)
}

/// The string builtins the incremental counting engine behind
/// [`MaterializedN3Graph`](crate::MaterializedN3Graph) evaluates itself (GH #6854).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CountedFunc {
    Concat,
    Scrape,
    EncodeForUri,
}

/// Whether the result of `f` over `args` (its argument list as substituted, or the one
/// subject of a unary builtin) is refused for its size: the text engine's own result
/// checkpoint ([`output_bytes`], a cheap bound and only past the limit the exact size),
/// run before the result is built. The cut and the run's refusal are recorded on `run`.
pub(crate) fn counted_result_refused(run: &Truncation, f: CountedFunc, args: &[Term]) -> bool {
    let f = match f {
        CountedFunc::Concat => Func::Concat,
        CountedFunc::Scrape => Func::Scrape,
        CountedFunc::EncodeForUri => Func::EncodeForUri,
    };
    // These builtins never read the store (only the list builtins walk a data list).
    let facts = FactIndex::default();
    let pending = bounded::Pending::default();
    let refused = refused_term(
        run,
        &pending,
        Derived::Result(f),
        output_bytes(f, args, &facts, &pending, false),
        || output_bytes(f, args, &facts, &pending, true),
    );
    settle(run, pending.finish(refused))
}

/// The [`bounded::term_bytes`] of the literal `true` that `log:conjunction` returns for the
/// empty formula.
fn conjunction_true_bytes() -> usize {
    bounded::TERM_NODE_BYTES + "true".len() + parser::XSD_BOOLEAN.len()
}

/// The [`bounded::term_bytes`] of the list builtin `args` member `a` stands for: a
/// first-class list, or the members of an rdf:first/rest data list walked from the store.
fn list_member_bytes(a: &Term, facts: &FactIndex, pending: &bounded::Pending) -> usize {
    let sum = |it: &mut dyn Iterator<Item = &Term>| {
        it.map(bounded::term_bytes).fold(0usize, usize::saturating_add)
    };
    match a {
        Term::List(ms) => sum(&mut ms.iter()),
        other => settle(pending, fact_list_refs(other, facts)).map_or(0, |ms| sum(&mut ms.into_iter())),
    }
}

/// A sink for the text a string builtin writes: the string itself, or (to size the
/// result without building it) a byte count.
trait TextOut {
    fn put(&mut self, s: &str);
}

impl TextOut for String {
    fn put(&mut self, s: &str) {
        self.push_str(s);
    }
}

/// Counts the bytes a string builtin would write.
#[derive(Default)]
struct TextBytes(usize);

impl TextOut for TextBytes {
    fn put(&mut self, s: &str) {
        self.0 = self.0.saturating_add(s.len());
    }
}

/// `time:timeZone`'s result: the explicit numeric offset of a dateTime's time part
/// (`Z` or no offset yields nothing, as in cwm), borrowed from the lexical form.
fn time_zone_offset(s: &str) -> Option<&str> {
    let t = s.split('T').nth(1)?;
    t.find(['+', '-']).map(|i| &t[i..])
}

/// The size of the term builtin `f` derives from `args`: with `exact`, its
/// [`bounded::term_bytes`], computed without building it; without, a cheap upper bound on
/// it from the input sizes. Every functional builtin has an entry (the match is
/// exhaustive). 0 for a builtin sized where it builds its result: `log:dtlit`,
/// `log:conclusion`, `log:content`, `log:parsedAsN3` / `log:semantics`, and every numeric
/// result, which is sized from its computed value before its literal is rendered
/// ([`numeric_result`], GH #6901).
fn output_bytes(
    f: Func,
    args: &[Term],
    facts: &FactIndex,
    pending: &bounded::Pending,
    exact: bool,
) -> usize {
    use bounded::{string_term_bytes, term_bytes, TERM_NODE_BYTES};
    let sum = |it: &mut dyn Iterator<Item = usize>| it.fold(0usize, usize::saturating_add);
    let lex0 = || args.first().and_then(lex).unwrap_or("");
    match f {
        Func::Concat if exact => string_term_bytes(sum(&mut args.iter().map(|a| concat_piece_len(a).unwrap_or(0)))),
        Func::Concat => string_term_bytes(sum(&mut args.iter().map(|a| match a {
            Term::Lit(v, dt, _) => concat_piece_bound(v, Some(dt)),
            Term::Iri(v) => concat_piece_bound(v, None),
            _ => 0,
        }))),
        Func::Format if exact => {
            let mut out = TextBytes::default();
            format_text(args, pending, &mut out).map_or(0, |()| string_term_bytes(out.0))
        }
        // `%d` / `%f` print any member read as a number, whatever its datatype.
        Func::Format => string_term_bytes(sum(&mut args.iter().map(|a| {
            lex(a).map_or(0, str::len).saturating_add(NUMERAL_TEXT_BYTES)
        }))),
        Func::Replace => match args {
            [Term::Lit(s, _, _), Term::Lit(pattern, _, _), Term::Lit(r, _, _)] if exact => {
                // A pattern that does not compile is the builtin's own cut.
                settle(pending, bounded::regex(pattern))
                    .map_or(0, |re| string_term_bytes(replace_bytes(&re, s, r)))
            }
            // Every match (at most one per position, plus one) is replaced by the
            // replacement's text, and every `$` reference in it expands to text inside a
            // match; matches do not overlap, so each reference adds at most the input.
            [Term::Lit(s, _, _), _, Term::Lit(r, _, _)] => {
                let refs = r.bytes().filter(|&c| c == b'$').count();
                let per_match = s.len().saturating_add(1).saturating_mul(r.len());
                string_term_bytes(
                    s.len()
                        .saturating_add(per_match)
                        .saturating_add(refs.saturating_mul(s.len())),
                )
            }
            _ => 0,
        },
        // Unicode case mapping and %XX-encoding at most triple the bytes.
        Func::LowerCase | Func::UpperCase | Func::EncodeForUri | Func::EncodeForUriCwm
        | Func::EncodeForFragId
            if !exact =>
        {
            string_term_bytes(lex0().len().saturating_mul(3))
        }
        Func::LowerCase => {
            string_term_bytes(sum(&mut lex0().chars().flat_map(char::to_lowercase).map(char::len_utf8)))
        }
        Func::UpperCase => {
            string_term_bytes(sum(&mut lex0().chars().flat_map(char::to_uppercase).map(char::len_utf8)))
        }
        Func::EncodeForUri => string_term_bytes(encode_for_uri_len(lex0())),
        Func::EncodeForUriCwm | Func::EncodeForFragId => {
            let keep_extra = cwm_uri_keeps(f);
            string_term_bytes(sum(&mut lex0().bytes().map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-') || keep_extra.contains(&c) {
                    1
                } else {
                    3
                }
            })))
        }
        // A merge of its members' lists (first-class, or data lists walked from the store).
        Func::Append => sum(&mut args.iter().map(|a| list_member_bytes(a, facts, pending)))
            .saturating_add(TERM_NODE_BYTES),
        // The distinct triples of its member formulae, or the literal `true` when there are
        // none (the empty formula).
        Func::Conjunction if exact => {
            let mut seen: FxHashSet<&[Term; 3]> = FxHashSet::default();
            let distinct: Vec<&[Term; 3]> = args
                .iter()
                .filter_map(|a| match a {
                    Term::Formula(ts) => Some(ts),
                    _ => None,
                })
                .flatten()
                .filter(|t| seen.insert(t))
                .collect();
            if distinct.is_empty() {
                conjunction_true_bytes()
            } else {
                sum(&mut distinct.into_iter().flatten().map(term_bytes)).saturating_add(TERM_NODE_BYTES)
            }
        }
        // A merged formula is at most its members together; with no triples it is `true`.
        Func::Conjunction => sum(&mut args.iter().map(term_bytes)).max(conjunction_true_bytes()),
        Func::Langlit => match args {
            [Term::Lit(lex, _, _), Term::Lit(lang, _, _)] => TERM_NODE_BYTES
                .saturating_add(lex.len())
                .saturating_add(RDF_LANG_STRING.len())
                .saturating_add(lang.len()),
            _ => 0,
        },
        // Sized where they are built: `log:dtlit` before its own early return,
        // `log:conclusion` from the borrowed closure, the parsed formula once parsed, and
        // `log:content`'s literal once the resolver has supplied its text.
        Func::Dtlit | Func::LogConclusion | Func::ParsedAsN3 | Func::Semantics | Func::Content => 0,
        // A member of the argument list, copied.
        Func::First => args.first().map_or(0, term_bytes),
        Func::Last => args.last().map_or(0, term_bytes),
        // The first capture group of the first match: at most the input; exactly, the
        // capture (the regex is compiled, and run, without building the result).
        Func::Scrape => match args {
            [Term::Lit(s, _, _), Term::Lit(pattern, _, _)] if exact => settle(pending, bounded::regex(pattern))
                .and_then(|re| re.captures(s).and_then(|c| c.get(1)).map(|m| m.len()))
                .map_or(0, string_term_bytes),
            _ => string_term_bytes(lex0().len()),
        },
        // The time zone offset: at most the input; exactly, the borrowed offset.
        Func::TimeZone if exact => time_zone_offset(lex0()).map_or(0, |o| string_term_bytes(o.len())),
        Func::TimeZone => string_term_bytes(lex0().len()),
        // A number, a count or a date part: sized from its value once computed, before its
        // literal is rendered ([`numeric_result`]).
        Func::Length
        | Func::StrLength
        | Func::MemberCount
        | Func::Sum
        | Func::Difference
        | Func::Product
        | Func::Quotient
        | Func::Remainder
        | Func::IntegerQuotient
        | Func::Max
        | Func::Min
        | Func::Exponentiation
        | Func::Logarithm
        | Func::Atan2
        | Func::Negation
        | Func::AbsoluteValue
        | Func::Rounded
        | Func::Floor
        | Func::Ceiling
        | Func::Sin
        | Func::Cos
        | Func::Tan
        | Func::Asin
        | Func::Acos
        | Func::Atan
        | Func::Sinh
        | Func::Cosh
        | Func::Tanh
        | Func::Asinh
        | Func::Acosh
        | Func::Atanh
        | Func::Degrees
        | Func::Radians
        | Func::Year
        | Func::Month
        | Func::Day
        | Func::Hours
        | Func::Minutes
        | Func::Seconds
        | Func::DayOfWeek
        | Func::InSeconds => 0,
    }
}

/// The bytes `regex`'s `replace_all(s, r)` writes, counted without building the output
/// or any expansion: each match is replaced by `r`'s literal text plus the captures it
/// names, under the regex crate's replacement syntax (`$$` is `$`; `$name` / `${name}` a
/// group by index or name, empty if it did not participate; a `$` that starts no
/// reference is literal).
fn replace_bytes<'r>(re: &regex::Regex, s: &str, r: &'r str) -> usize {
    enum Piece<'r> {
        Text(usize),
        Index(usize),
        Name(&'r str),
    }
    let valid = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // A reference by number, else by name.
    let reference = |name: &'r str| name.parse::<usize>().map_or(Piece::Name(name), Piece::Index);
    let mut pieces = Vec::new();
    let mut rest = r;
    while let Some(i) = rest.find('$') {
        pieces.push(Piece::Text(i));
        let after = &rest[i + 1..];
        if let Some(tail) = after.strip_prefix('$') {
            pieces.push(Piece::Text(1));
            rest = tail;
        } else if let Some(braced) = after.strip_prefix('{') {
            match braced.find('}') {
                Some(end) => {
                    pieces.push(reference(&braced[..end]));
                    rest = &braced[end + 1..];
                }
                None => {
                    pieces.push(Piece::Text(1));
                    rest = after;
                }
            }
        } else {
            let end = after.bytes().position(|c| !valid(c)).unwrap_or(after.len());
            if end == 0 {
                pieces.push(Piece::Text(1));
            } else {
                pieces.push(reference(&after[..end]));
            }
            rest = &after[end..];
        }
    }
    pieces.push(Piece::Text(rest.len()));
    let mut out = s.len();
    for caps in re.captures_iter(s) {
        let matched = caps.get(0).map_or(0, |m| m.len());
        let replaced = pieces
            .iter()
            .map(|p| match p {
                Piece::Text(n) => *n,
                Piece::Index(i) => caps.get(*i).map_or(0, |m| m.len()),
                Piece::Name(n) => caps.name(n).map_or(0, |m| m.len()),
            })
            .fold(0usize, usize::saturating_add);
        out = out.saturating_sub(matched).saturating_add(replaced);
    }
    out
}

/// ( fmt args… ) string:format: the honest %s/%d/%f/%% subset, written to `out`; `None`
/// for any other directive or an argument-count mismatch (which EYE throws on).
fn format_text(args: &[Term], pending: &bounded::Pending, out: &mut impl TextOut) -> Option<()> {
    let fmt = lex(args.first()?)?;
    let mut argi = 1;
    let mut chars = fmt.chars();
    let mut buf = [0u8; 4];
    while let Some(c) = chars.next() {
        if c != '%' {
            out.put(c.encode_utf8(&mut buf));
            continue;
        }
        match chars.next()? {
            '%' => out.put("%"),
            's' => {
                out.put(lex(args.get(argi)?)?);
                argi += 1;
            }
            'd' => {
                // The integer part: exact in the exact tower, else from the f64.
                let n = match numval_in(args.get(argi)?, pending)? {
                    NumVal::Int(i) => i,
                    NumVal::Dec(m, sc) => m / rep(pending, 10i128.checked_pow(sc))?,
                    NumVal::F64(f) => i128::from(settle(pending, bounded::int_of_f64(f))?),
                };
                out.put(&n.to_string());
                argi += 1;
            }
            'f' => {
                // C printf's default precision, 6 digits: exact from the exact
                // tower, else from the f64 (a numeral past i128 is a cut).
                out.put(&fixed6(numval_in(args.get(argi)?, pending)?, pending)?);
                argi += 1;
            }
            _ => return None, // unsupported directive: fail, don't mangle
        }
    }
    (argi == args.len()).then_some(())
}

/// The text `string:concatenation` takes from member `a`: cwm coerces typed literals to
/// their canonical VALUE string ("0"^^xsd:boolean → "false", 0E1 → "0") and IRIs to their
/// text (concatenation.n3 s01); `None` for any other member.
fn concat_piece(a: &Term) -> Option<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    Some(match a {
        Term::Lit(v, dt, _) => match dt.strip_prefix("http://www.w3.org/2001/XMLSchema#") {
            Some("boolean") => Cow::Borrowed(if v == "0" || v == "false" { "false" } else { "true" }),
            Some("integer" | "decimal" | "float" | "double") => match numval(a) {
                // A numeral past i128: its canonical value string, exactly.
                Some(NumVal::F64(_)) if big_numeral_canonical(v).is_some() => {
                    Cow::Owned(big_numeral_canonical(v)?)
                }
                Some(NumVal::Int(i)) => Cow::Owned(i.to_string()),
                Some(NumVal::Dec(m, sc)) => {
                    let (m, sc) = dec_norm(m, sc);
                    if sc == 0 {
                        Cow::Owned(m.to_string())
                    } else {
                        let Term::Lit(lex, _, _) = numval_term(NumVal::Dec(m, sc)) else { return None };
                        Cow::Owned(lex)
                    }
                }
                Some(NumVal::F64(f)) => {
                    if number_is_whole(f) {
                        // not-a-cut: exact-cast (whole, below 2^53)
                        Cow::Owned((f as i64).to_string())
                    } else {
                        Cow::Owned(format!("{f}"))
                    }
                }
                None => Cow::Borrowed(v),
            },
            _ => Cow::Borrowed(v),
        },
        Term::Iri(i) => Cow::Borrowed(i),
        _ => return None,
    })
}

/// A cheap upper bound on the length of [`concat_piece`]'s text for a member with lexical
/// form `lex` and datatype `dt` (`None` for an IRI): a typed literal other than
/// `xsd:string` may print as its canonical value, up to [`NUMERAL_TEXT_BYTES`] past its
/// lexical form. Shared by the text engine and the compiled evaluator.
fn concat_piece_bound(lex: &str, dt: Option<&str>) -> usize {
    match dt.and_then(|dt| dt.strip_prefix("http://www.w3.org/2001/XMLSchema#")) {
        Some(local) if local != "string" => lex.len().saturating_add(NUMERAL_TEXT_BYTES),
        _ => lex.len(),
    }
}

/// The length of [`encode_for_uri`]'s result for `s`, computed without building it.
fn encode_for_uri_len(s: &str) -> usize {
    s.bytes().fold(0usize, |n, c| {
        n.saturating_add(if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~') { 1 } else { 3 })
    })
}

/// The length of [`concat_piece`]'s text for member `a`, computed from the value without
/// building it (GH #6901).
fn concat_piece_len(a: &Term) -> Option<usize> {
    Some(match a {
        Term::Lit(v, dt, _) => match dt.strip_prefix("http://www.w3.org/2001/XMLSchema#") {
            Some("boolean") => {
                if v == "0" || v == "false" {
                    5
                } else {
                    4
                }
            }
            Some("integer" | "decimal" | "float" | "double") => match numval(a) {
                Some(NumVal::F64(f)) => big_numeral_canonical_len(v).unwrap_or_else(|| number_lex_len(f)),
                Some(NumVal::Int(i)) => int_lex_len(i),
                Some(NumVal::Dec(m, sc)) => {
                    let (m, sc) = dec_norm(m, sc);
                    if sc == 0 {
                        int_lex_len(m)
                    } else {
                        dec_lex_len(m, sc)
                    }
                }
                None => v.len(),
            },
            _ => v.len(),
        },
        Term::Iri(i) => i.len(),
        _ => return None,
    })
}

/// The extra bytes cwm's `string:encodeForURI` / `string:encodeForFragID` keep unencoded
/// (uriEncode-out.n3): URI keeps #'()~ but encodes '/'; FragID keeps '/' but encodes #'()~.
fn cwm_uri_keeps(f: Func) -> &'static [u8] {
    if matches!(f, Func::EncodeForUriCwm) {
        b"#'()~"
    } else {
        b"/"
    }
}

/// Evaluate a functional builtin `(members) op object`: resolve the list members under `b`,
/// compute, then either bind the object variable to the result or filter if it is ground.
/// Evaluate a functional builtin. Every fallible step inside settles into a local
/// [`bounded::Pending`], so the caller gets a [`Bounded`] it must settle on the run.
fn eval_functional(
    f: Func,
    subj: &Term,
    obj: &Term,
    facts: &FactIndex,
    bw: &BwCtx,
    b: Binding,
) -> Bounded<Option<Binding>> {
    let pending = bounded::Pending::default();
    let v = eval_functional_inner(f, subj, obj, facts, bw, b, &pending);
    pending.finish(v)
}

fn eval_functional_inner(
    f: Func,
    subj: &Term,
    obj: &Term,
    facts: &FactIndex,
    bw: &BwCtx,
    b: Binding,
    pending: &bounded::Pending,
) -> Option<Binding> {
    const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
    // log:dtlit needs the UNAPPLIED member terms: its reverse mode binds them by
    // decomposing a ground object literal into ( "lexical" datatype-IRI ).
    if let Func::Dtlit = f {
        let Term::List(members) = subj else { return None };
        if members.len() != 2 {
            return None;
        }
        let run = &bw.truncated;
        // The members as bound, borrowed: the forward result is sized before any copy.
        let resolve = |t: &Term| -> Option<(usize, bool)> {
            let r = match t {
                Term::Var(v) => b.get(v)?,
                other => other,
            };
            match r {
                Term::Lit(v, _, _) => Some((v.len(), true)),
                Term::Iri(i) => Some((i.len(), false)),
                _ => None,
            }
        };
        if let (Some((lex, true)), Some((dt, false))) = (resolve(&members[0]), resolve(&members[1])) {
            let bytes = bounded::TERM_NODE_BYTES.saturating_add(lex).saturating_add(dt);
            if refused_term(&bw.truncated, pending, Derived::Result(f), bytes, || bytes) {
                return None;
            }
        }
        let (m0, m1) = (bounded::substitute(&members[0], &b, run)?, bounded::substitute(&members[1], &b, run)?);
        let mut nb = b;
        return match (m0, m1) {
            // forward: ( "lex" xsd:dt ) → "lex"^^xsd:dt (sized above; the parts are moved)
            (Term::Lit(v, _, _), Term::Iri(dt)) => {
                let lit = Term::Lit(v, dt, None);
                unify_term(obj, &lit, &mut nb, &bw.truncated).then_some(nb)
            }
            // reverse: decompose a ground object literal into its parts
            // (each part sized from the borrowed literal before it is copied)
            _ => {
                let bound = match obj {
                    Term::Var(v) => nb.get(v)?,
                    other => other,
                };
                let Term::Lit(v, dt, _) = bound else { return None };
                let bytes = bounded::string_term_bytes(v.len())
                    .max(bounded::TERM_NODE_BYTES.saturating_add(dt.len()));
                if refused_term(run, pending, Derived::Result(f), bytes, || bytes) {
                    return None;
                }
                let (lex, dt) = (Term::Lit(v.clone(), XSD_STRING.into(), None), Term::Iri(dt.clone()));
                (unify_term(&members[0], &lex, &mut nb, &bw.truncated) && unify_term(&members[1], &dt, &mut nb, &bw.truncated))
                    .then_some(nb)
            }
        };
    }
    // log:conclusion closes the subject formula as bound, borrowed (not copied into an
    // argument list). The closure ran under the run's limits; the formula holding all of
    // it is one more term, sized from the borrowed closure before its statements are
    // copied.
    if let Func::LogConclusion = f {
        let bound = match subj {
            Term::Var(v) => b.get(v)?,
            other => other,
        };
        let Term::Formula(ts) = bound else { return None };
        let result = with_formula_closure(ts, bw, |all| {
            let bytes = formula_bytes(all.iter().copied());
            (!refused_term(&bw.truncated, pending, Derived::Result(f), bytes, || bytes))
                .then(|| Term::Formula(all.iter().map(|t| (*t).clone()).collect()))
        })?;
        let mut nb = b;
        return unify_term(obj, &result, &mut nb, &bw.truncated).then_some(nb);
    }
    // math:negation is bidirectional in EYE: `?x math:negation 3` solves
    // ?x = -3 (the one reverse mode the suites rely on).
    if matches!(f, Func::Negation) && !subj.is_ground() && !matches!(subj, Term::List(_)) {
        let s_applied = bounded::substitute(subj, &b, &bw.truncated)?;
        if !s_applied.is_ground() {
            let o_applied = bounded::substitute(obj, &b, &bw.truncated)?;
            if let Some(v) = numval_in(&o_applied, pending) {
                let negated = rep(pending, numval_negate(v))?;
                // Sized from the value before it is rendered.
                let t = numeric_result(&bw.truncated, pending, f, Numeral::Num(negated))?;
                let mut nb = b;
                return unify_term(subj, &t, &mut nb, &bw.truncated).then_some(nb);
            }
            return None;
        }
    }
    // Reverse (object-bound) modes of the invertible unary builtins:
    // `?y math:sin 0` solves y = asin 0 (cwm trig.n3 test4), and
    // `?t time:inSeconds N` formats the epoch back to a UTC dateTime.
    // The subject with the binding substituted, computed once: it may be a large bound
    // string, and each copy is held while the result is sized.
    let subj_applied = match subj {
        Term::List(_) => None,
        _ => Some(bounded::substitute(subj, &b, &bw.truncated)?),
    };
    if !subj.is_ground() && subj_applied.as_ref().is_some_and(|s| !s.is_ground()) {
        let o_applied = bounded::substitute(obj, &b, &bw.truncated)?;
        if o_applied.is_ground() {
            // Each result below is sized from its value before it is rendered.
            let inverse = |v: f64| -> Option<f64> {
                Some(match f {
                    Func::Sin => v.asin(),
                    Func::Cos => v.acos(),
                    Func::Tan => v.atan(),
                    Func::Asin => v.sin(),
                    Func::Acos => v.cos(),
                    Func::Atan => v.tan(),
                    Func::Sinh => v.asinh(),
                    Func::Cosh => v.acosh(),
                    Func::Tanh => v.atanh(),
                    Func::Asinh => v.sinh(),
                    Func::Acosh => v.cosh(),
                    Func::Atanh => v.tanh(),
                    Func::Degrees => v * std::f64::consts::PI / 180.0,
                    Func::Radians => v * 180.0 / std::f64::consts::PI,
                    _ => return None,
                })
            };
            if let Func::InSeconds = f {
                // Exact: an epoch second count is a whole number, never an f64 image.
                let secs = numval_whole_i64(numval_in(&o_applied, pending)?, pending)?;
                let bytes = bounded::string_term_bytes(epoch_len(secs));
                if refused_term(&bw.truncated, pending, Derived::Result(f), bytes, || bytes) {
                    return None;
                }
                let mut nb = b;
                let lit = Term::Lit(format_epoch(secs), XSD_STRING.into(), None);
                return unify_term(subj, &lit, &mut nb, &bw.truncated).then_some(nb);
            }
            // not-a-cut: defined-float (the inverse trig functions)
            if let Some(x) = numval_in(&o_applied, pending).map(NumVal::to_f64) {
                if let Some(v) = inverse(x) {
                    if v.is_nan() {
                        return None;
                    }
                    let t = numeric_result(&bw.truncated, pending, f, Numeral::Double(v))?;
                    let mut nb = b;
                    return unify_term(subj, &t, &mut nb, &bw.truncated).then_some(nb);
                }
            }
            return None;
        }
    }
    // Arguments: the list members (rule-local structure, a data list walked
    // from the fact store, or `rdf:nil` = the empty list), else a singleton
    // for the unary (math:/string:/time:) ops.
    // A list is sized from the borrowed terms (its members with the binding substituted,
    // or the data list's members in the store) BEFORE it is built: a rule-local list can
    // name a bound term many times, and a data list copies its members (GH #6771).
    let subj_bound = match subj {
        Term::Var(v) => b.get(v).unwrap_or(subj),
        _ => subj,
    };
    let run = &bw.truncated;
    let (args, was_list): (Vec<Term>, bool) = if let Term::List(_) = subj_bound {
        // First-class list value (substituted, and sized first, by `substitute`).
        let applied = match subj_applied {
            Some(t) => t,
            None => bounded::substitute(subj, &b, run)?,
        };
        let Term::List(members) = applied else { return None };
        (members, true)
    } else {
        let subj_applied = subj_applied?;
        // A data list written as rdf:first/rest triples, via a bound variable.
        match settle(pending, fact_list_refs(&subj_applied, facts)) {
            Some(members) => {
                let bytes = members
                    .iter()
                    .map(|m| bounded::resolved_bytes(m, &b, false))
                    .fold(bounded::TERM_NODE_BYTES, usize::saturating_add);
                if refused_term(&bw.truncated, pending, Derived::Arguments(f), bytes, || bytes) {
                    return None;
                }
                let members = members
                    .iter()
                    .map(|m| bounded::substitute(m, &b, run))
                    .collect::<Option<Vec<Term>>>()?;
                (members, true)
            }
            None => (vec![subj_applied], false),
        }
    };
    // The list:-namespace ops are only defined ON lists.
    if matches!(f, Func::Length | Func::First | Func::Last | Func::Append) && !was_list {
        return None;
    }
    if args.is_empty()
        && !matches!(
            f,
            Func::Conjunction
                | Func::MemberCount
                | Func::Length
                | Func::Append
                | Func::Sum
                | Func::Product
                | Func::Concat
        )
    {
        return None;
    }
    // The result is sized from the arguments BEFORE it is built, so a term that grows
    // every round is refused (a cut, and an error that fails the closure) instead of
    // exhausting memory (GH #6771).
    if refused_term(
        &bw.truncated,
        pending,
        Derived::Result(f),
        output_bytes(f, &args, facts, pending, false),
        || output_bytes(f, &args, facts, pending, true),
    ) {
        return None;
    }
    let result: Term = match f {
        Func::Conjunction => {
            // Merge a list of formulae (EYE conjoin): duplicates deduped, order preserved.
            // The boolean literal `true` counts as the empty formula (EYE's conjoin(true)).
            let mut merged: Vec<[Term; 3]> = Vec::new();
            for a in &args {
                match a {
                    Term::Formula(ts) => {
                        for t in ts {
                            if !merged.contains(t) {
                                merged.push(t.clone());
                            }
                        }
                    }
                    Term::Lit(v, _, _) if v == "true" => {}
                    _ => return None,
                }
            }
            if merged.is_empty() {
                // the empty formula IS the literal true
                Term::Lit("true".into(), parser::XSD_BOOLEAN.into(), None)
            } else {
                Term::Formula(merged)
            }
        }
        Func::MemberCount => match &args[..] {
            // a `( … )` list: its length; a quoted formula: its DISTINCT triple count
            [Term::Formula(ts)] => {
                let distinct: FxHashSet<&[Term; 3]> = ts.iter().collect();
                numeric_result(run, pending, f, Numeral::count(distinct.len()))?
            }
            _ if was_list => numeric_result(run, pending, f, Numeral::count(args.len()))?,
            _ => return None,
        },
        Func::Format => {
            let mut out = String::new();
            format_text(&args, pending, &mut out)?;
            Term::Lit(out, XSD_STRING.into(), None)
        }
        Func::Scrape => {
            // ( str regex ) string:scrape ?out — first capture group of the first match.
            if args.len() != 2 {
                return None;
            }
            let re = settle(pending, bounded::regex(lex(&args[1])?))?;
            let cap = re.captures(lex(&args[0])?)?.get(1)?.as_str().to_string();
            Term::Lit(cap, XSD_STRING.into(), None)
        }
        Func::Concat => {
            let mut s = String::new();
            for a in &args {
                s.push_str(&concat_piece(a)?);
            }
            Term::Lit(s, "http://www.w3.org/2001/XMLSchema#string".into(), None)
        }
        Func::Length => numeric_result(run, pending, f, Numeral::count(args.len()))?,
        Func::StrLength => numeric_result(run, pending, f, Numeral::count(lex(&args[0])?.chars().count()))?,
        Func::LowerCase => {
            Term::Lit(lex(&args[0])?.to_lowercase(), "http://www.w3.org/2001/XMLSchema#string".into(), None)
        }
        Func::UpperCase => {
            Term::Lit(lex(&args[0])?.to_uppercase(), "http://www.w3.org/2001/XMLSchema#string".into(), None)
        }
        Func::EncodeForUri => {
            Term::Lit(encode_for_uri(lex(&args[0])?), "http://www.w3.org/2001/XMLSchema#string".into(), None)
        }
        Func::EncodeForUriCwm | Func::EncodeForFragId => {
            // cwm's quoting pairs (uriEncode-out.n3): URI keeps #'()~ but
            // encodes '/'; FragID keeps '/' but encodes #'()~. Both keep
            // alphanumerics and _.- and use uppercase hex.
            let keep_extra = cwm_uri_keeps(f);
            let mut out = String::new();
            for byte in lex(&args[0])?.bytes() {
                match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' => {
                        out.push(byte as char)
                    }
                    c if keep_extra.contains(&c) => out.push(c as char),
                    c => out.push_str(&format!("%{c:02X}")),
                }
            }
            Term::Lit(out, "http://www.w3.org/2001/XMLSchema#string".into(), None)
        }
        // Evaluated above, from the borrowed subject.
        Func::LogConclusion => return None,
        Func::Semantics | Func::Content => match &args[..] {
            [Term::Iri(doc)] => {
                // No resolver (document access is off for this run) leaves the document's
                // content as unknown as a resolver that cannot supply it: both are cuts.
                let text = settle(pending, bounded::resolved(bw.resolver.and_then(|r| r(doc))))?;
                if matches!(f, Func::Content) {
                    // The text is the resolver's; the literal holding it is sized first.
                    let bytes = bounded::string_term_bytes(text.len());
                    if refused_term(&bw.truncated, pending, Derived::Result(f), bytes, || bytes) {
                        return None;
                    }
                    Term::Lit(text, XSD_STRING.into(), None)
                } else {
                    // cwm-faithful: `log:semantics` returns the document's PARSED statements
                    // (rules re-encoded as `=>` triples), NOT their closure — the closure is
                    // taken later by `log:supports` / `log:conclusion`, which is where the
                    // import-cycle guard ([`VisitedDocs`]) applies. Resolution itself does no
                    // recursion, so no marking is needed here.
                    parsed_formula(f, &text, doc, bw, pending)?
                }
            }
            _ => return None,
        },
        Func::ParsedAsN3 => match &args[..] {
            [Term::Lit(src, _, _)] => {
                parsed_formula(f, src, &bw.base, bw, pending)?
            }
            _ => return None,
        },
        Func::Langlit => match &args[..] {
            [Term::Lit(lex, _, _), Term::Lit(lang, _, _)] => Term::Lit(
                lex.clone(),
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".into(),
                Some(lang.clone()),
            ),
            _ => return None,
        },
        Func::First => args.first()?.clone(),
        Func::Last => args.last()?.clone(),
        Func::Append => {
            // ( l1 l2 … ) list:append ?out — concatenate; every member must
            // itself be a list (first-class, or data first/rest structure).
            let mut merged: Vec<Term> = Vec::new();
            for a in &args {
                match a {
                    Term::List(ms) => merged.extend(ms.iter().cloned()),
                    other => merged.extend(settle(pending, fact_list(other, facts))?),
                }
            }
            Term::List(merged)
        }
        Func::Replace => {
            // ( str pattern replacement ) string:replace ?out — regex replace-all.
            if args.len() != 3 {
                return None;
            }
            let re = settle(pending, bounded::regex(lex(&args[1])?))?;
            let out = re.replace_all(lex(&args[0])?, lex(&args[2])?).into_owned();
            Term::Lit(out, "http://www.w3.org/2001/XMLSchema#string".into(), None)
        }
        Func::Year
        | Func::Month
        | Func::Day
        | Func::Hours
        | Func::Minutes
        | Func::Seconds
        | Func::DayOfWeek
        | Func::InSeconds => {
            let part = i128::from(datetime_part(lex(&args[0])?, f, pending)?);
            numeric_result(run, pending, f, Numeral::Num(NumVal::Int(part)))?
        }
        Func::TimeZone => {
            // Only an EXPLICIT numeric offset is a time zone (cwm: `Z`/absent
            // yield nothing).
            let off = time_zone_offset(lex(&args[0])?)?;
            Term::Lit(off.to_string(), XSD_STRING.into(), None)
        }
        _ => {
            // Unary numeric/trig/time builtins take a DIRECT value, never a
            // `( … )` list — `(1) math:rounded ?x` must fail (cwm/EYE agree;
            // the suites assert it via :FAILURE rules).
            let unary = matches!(
                f,
                Func::Negation
                    | Func::AbsoluteValue
                    | Func::Rounded
                    | Func::Floor
                    | Func::Ceiling
                    | Func::Sin
                    | Func::Cos
                    | Func::Tan
                    | Func::Asin
                    | Func::Acos
                    | Func::Atan
                    | Func::Sinh
                    | Func::Cosh
                    | Func::Tanh
                    | Func::Asinh
                    | Func::Acosh
                    | Func::Atanh
                    | Func::Degrees
                    | Func::Radians
            );
            if unary && was_list {
                return None;
            }
            // An exact step that overflows is a cut (settled here); the f64 path below
            // still gives the approximate value, as before.
            if let Some(exact) = settle(pending, eval_exact(f, &args)) {
                numeric_result(run, pending, f, Numeral::Num(exact))?
            } else {
                let nvals: Vec<NumVal> = args
                    .iter()
                    .map(|a| numval_in(a, pending))
                    .collect::<Option<_>>()?;
                // Doubles, the trig/log family or a non-terminating quotient; an exact
                // step that overflowed was a cut above.
                // not-a-cut: defined-float
                let nums: Vec<f64> = nvals.iter().map(|v| v.to_f64()).collect();
                // cwm/EYE type discipline: the real-valued (trig/log) family is
                // ALWAYS double; arithmetic is double when any input is.
                let trig_family = matches!(
                    f,
                    Func::Sin
                        | Func::Cos
                        | Func::Tan
                        | Func::Asin
                        | Func::Acos
                        | Func::Atan
                        | Func::Sinh
                        | Func::Cosh
                        | Func::Tanh
                        | Func::Asinh
                        | Func::Acosh
                        | Func::Atanh
                        | Func::Degrees
                        | Func::Radians
                        | Func::Logarithm
                        | Func::Atan2
                );
                let any_double = nvals.iter().any(|v| matches!(v, NumVal::F64(_)))
                    || args.iter().any(|a| {
                        matches!(a, Term::Lit(_, dt, _)
                            if dt == "http://www.w3.org/2001/XMLSchema#double"
                                || dt == "http://www.w3.org/2001/XMLSchema#float")
                    });
                let two = |n: &[f64]| if n.len() == 2 { Some((n[0], n[1])) } else { None };
                let v = match f {
                    Func::Sum => nums.iter().sum(),
                    Func::Product => nums.iter().product(),
                    Func::Max => nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    Func::Min => nums.iter().copied().fold(f64::INFINITY, f64::min),
                    Func::Difference => {
                        let (a, b) = two(&nums)?;
                        a - b
                    }
                    Func::Quotient => {
                        let (a, b) = two(&nums)?;
                        if b == 0.0 && !any_double {
                            return None; // exact-arithmetic division by zero fails
                        }
                        a / b // IEEE for doubles: ±INF / NaN (cwm math/inf.n3 test5)
                    }
                    Func::Exponentiation => {
                        let (a, b) = two(&nums)?;
                        a.powf(b)
                    }
                    Func::Logarithm => {
                        // (x base) → log_base(x); EYE computes log(U)/log(V).
                        let (x, base) = two(&nums)?;
                        if x <= 0.0 || base <= 0.0 || base == 1.0 {
                            return None;
                        }
                        x.ln() / base.ln()
                    }
                    Func::Atan2 => {
                        // EYE's eye.pl: `W is atan(U/V)` — deliberately matched (see module doc).
                        let (x, y) = two(&nums)?;
                        if y == 0.0 {
                            return None;
                        }
                        (x / y).atan()
                    }
                    Func::Remainder => return None, // integer-only (cwm); handled in eval_exact
                    Func::IntegerQuotient => {
                        let (a, b) = two(&nums)?;
                        if b == 0.0 {
                            return None;
                        }
                        (a / b).floor()
                    }
                    Func::Negation => -nums[0],
                    Func::AbsoluteValue => nums[0].abs(),
                    Func::Rounded => (nums[0] + 0.5).floor(), // round-half-UP (suite refs)
                    Func::Floor => nums[0].floor(),
                    Func::Ceiling => nums[0].ceil(),
                    Func::Sin => nums[0].sin(),
                    Func::Cos => nums[0].cos(),
                    Func::Tan => nums[0].tan(),
                    Func::Asin => nums[0].asin(),
                    Func::Acos => nums[0].acos(),
                    Func::Atan => nums[0].atan(),
                    Func::Sinh => nums[0].sinh(),
                    Func::Cosh => nums[0].cosh(),
                    Func::Tanh => nums[0].tanh(),
                    Func::Asinh => nums[0].asinh(),
                    Func::Acosh => nums[0].acosh(),
                    Func::Atanh => nums[0].atanh(),
                    Func::Degrees => nums[0] * 180.0 / std::f64::consts::PI,
                    Func::Radians => nums[0] * std::f64::consts::PI / 180.0,
                    _ => unreachable!(),
                };
                if v.is_nan()
                    && !nums.iter().any(|x| x.is_nan() || x.is_infinite())
                    && !(matches!(f, Func::Quotient) && any_double)
                {
                    return None; // domain error (asin 2, acosh 0.5, …): the premise fails
                    // (NaN/INF inputs — and IEEE 0/0 — PROPAGATE instead, cwm math/inf.n3)
                }
                let n = if trig_family || any_double { Numeral::Double(v) } else { Numeral::Num(NumVal::F64(v)) };
                numeric_result(run, pending, f, n)?
            }
        }
    };
    // Bind the object variable; if it is already GROUND, compare NUMERICALLY
    // when both sides are numbers, under the same XPath type promotion as the
    // `math:` comparisons (GH #6745: so `15.5` matches a computed `15.5e0` and
    // exact-decimal results match their reference lexical forms), else
    // structurally.
    let mut nb = b;
    let obj_applied = bounded::substitute(obj, &nb, &bw.truncated)?;
    if obj_applied.is_ground() {
        if let (Some(x), Some(y)) = (cmp_num(&obj_applied, true), cmp_num(&result, true)) {
            return (cmp_promoted(x, y) == Some(std::cmp::Ordering::Equal)).then_some(nb);
        }
    }
    if unify_term(obj, &result, &mut nb, &bw.truncated) {
        Some(nb)
    } else {
        None
    }
}

/// Exact evaluation of the arithmetic builtins when every argument is an
/// integer or decimal (scaled-i128): returns `None` to fall back to f64
/// (doubles involved, overflow, or a non-exact quotient). The RESULT TYPE
/// follows EYE: all-integer in → integer out; any decimal in → decimal out
/// (with at least one fraction digit in the lexical form).
fn eval_exact(f: Func, args: &[Term]) -> Bounded<Option<NumVal>> {
    let pending = bounded::Pending::default();
    let v = eval_exact_inner(f, args, &pending);
    pending.finish(v)
}

/// One step of exact arithmetic: `None` from a checked operation means the exact value
/// exists but `i128` cannot hold it, which is a cut (see [`bounded::exact`]).
fn rep<T>(pending: &bounded::Pending, step: Option<T>) -> Option<T> {
    settle(pending, bounded::exact(step))
}

fn eval_exact_inner(f: Func, args: &[Term], pending: &bounded::Pending) -> Option<NumVal> {
    let vals: Vec<NumVal> = args
        .iter()
        .map(|a| numval_in(a, pending))
        .collect::<Option<_>>()?;
    if vals.iter().any(|v| matches!(v, NumVal::F64(_))) {
        return None;
    }
    let any_dec = vals.iter().any(|v| matches!(v, NumVal::Dec(_, _)));
    let pair = || -> Option<(i128, i128, u32)> {
        if vals.len() == 2 {
            rep(pending, NumVal::aligned(vals[0], vals[1]))
        } else {
            None
        }
    };
    let renorm = |m: i128, s: u32| -> NumVal {
        if any_dec {
            NumVal::Dec(m, s)
        } else {
            NumVal::Int(m) // s is 0 when no decimals were involved
        }
    };
    // Unary ops: value = m / 10^s; integer-valued results keep the input's
    // numeric type (decimal in → `x.0` out, matching the cwm references).
    let unary_int = |round: fn(i128, i128) -> Option<i128>| -> Option<NumVal> {
        let (m, s) = match vals[0] {
            NumVal::Int(i) => (i, 0u32),
            NumVal::Dec(m, s) => (m, s),
            NumVal::F64(_) => return None,
        };
        let pow = rep(pending, 10i128.checked_pow(s))?;
        let v = rep(pending, round(m, pow))?;
        Some(if s == 0 { NumVal::Int(v) } else { NumVal::Dec(v, 0) })
    };
    // The exact add / subtract / multiply DELEGATE to the shared substrate
    // `Dec` (byte-identical `(mant, scale)`: `+`/`-` keep the max scale, `*` sums
    // the scales — the SAME i128 mantissa ops the private tower did). `renorm`
    // keeps EYE's result-type rule (all-integer in → integer out; any decimal in
    // → decimal out). Every checked step goes through `rep`, so an overflow is a cut.
    use sparq_substrate::numeric::Dec as SubDec;
    let out = match f {
        Func::Sum => {
            let mut acc = SubDec { mant: 0, scale: 0 };
            for &v in &vals {
                acc = rep(pending, acc.checked_add(numval_to_subdec(v)?))?;
            }
            renorm(acc.mant, acc.scale)
        }
        Func::Product => {
            let mut acc = SubDec { mant: 1, scale: 0 };
            for &v in &vals {
                acc = rep(pending, acc.checked_mul(numval_to_subdec(v)?))?;
            }
            renorm(acc.mant, acc.scale)
        }
        Func::Difference => {
            if vals.len() != 2 {
                return None;
            }
            let (x, y) = (numval_to_subdec(vals[0])?, numval_to_subdec(vals[1])?);
            let d = rep(pending, x.checked_sub(y))?;
            renorm(d.mant, d.scale)
        }
        Func::Max | Func::Min => {
            // Compare via the shared substrate `Dec::cmp` (scale-aligned i128 order).
            // The WINNING ORIGINAL operand is returned unchanged (its own scale kept).
            let mut best = vals[0];
            for &v in &vals[1..] {
                let ord = rep(pending, numval_to_subdec(best)?.cmp(numval_to_subdec(v)?))?;
                let take = if matches!(f, Func::Max) {
                    ord == std::cmp::Ordering::Less
                } else {
                    ord == std::cmp::Ordering::Greater
                };
                if take {
                    best = v;
                }
            }
            best
        }
        Func::Quotient => {
            // Long-divide to an exact decimal if one exists within i128 range.
            let (mut a, b, _) = pair()?;
            if b == 0 {
                return None; // division by zero: the builtin is undefined (no-match)
            }
            let mut scale = 0u32;
            // `wrapping_rem` is the exact remainder for every non-zero divisor
            // (`i128::MIN % -1` is 0; only the quotient overflows there).
            while a.wrapping_rem(b) != 0 && scale < 34 {
                a = rep(pending, a.checked_mul(10))?;
                scale += 1;
            }
            if a.wrapping_rem(b) != 0 {
                return None; // a non-terminating decimal: EYE's double result (f64 path)
            }
            let q = rep(pending, a.checked_div(b))?;
            if any_dec || scale > 0 {
                NumVal::Dec(q, scale)
            } else {
                NumVal::Int(q)
            }
        }
        Func::Remainder => {
            // INTEGER-only (cwm remainder.n3: any non-integer operand FAILS),
            // with the sign of the DIVISOR (Python %, matching the cwm refs:
            // -2 mod 4 = 2, 2 mod -4 = -2).
            if vals.len() != 2 {
                return None;
            }
            match (vals[0], vals[1]) {
                (NumVal::Int(a), NumVal::Int(b)) if b != 0 => {
                    // Exact for every non-zero divisor, `i128::MIN % -1` (= 0) included.
                    let r = a.wrapping_rem(b);
                    if r != 0 && (r < 0) != (b < 0) {
                        NumVal::Int(rep(pending, r.checked_add(b))?)
                    } else {
                        NumVal::Int(r)
                    }
                }
                _ => return None,
            }
        }
        Func::IntegerQuotient => {
            let (a, b, _) = pair()?;
            if b == 0 {
                return None;
            }
            NumVal::Int(rep(pending, a.checked_div_euclid(b))?)
        }
        // Tier-preserving unary sign ops (i128 `checked_neg`/`checked_abs`; `i128::MIN`
        // has no i128 negation, which is a cut).
        Func::Negation => match vals[0] {
            NumVal::F64(_) => return None,
            v => rep(pending, numval_negate(v))?,
        },
        Func::AbsoluteValue => match vals[0] {
            NumVal::F64(_) => return None,
            v => rep(pending, numval_abs(v))?,
        },
        // round-half-UP: floor(x + 1/2) — what the suite references encode
        // (-2.5 → -2, 0.5 → 1, 2.5 → 3). rounded keeps the decimal TYPE
        // (`-3.0`), while floor/ceiling return integers — both per the cwm
        // reference outputs.
        // not-a-cut: settled-by-caller (`unary_int` settles each rounding step)
        Func::Rounded => unary_int(|m, pow| Some(m.checked_add(pow / 2)?.div_euclid(pow)))?,
        Func::Floor => match unary_int(|m, pow| Some(m.div_euclid(pow)))? {
            NumVal::Dec(m, _) => NumVal::Int(m),
            v => v,
        },
        Func::Ceiling => {
            // not-a-cut: settled-by-caller (`unary_int` settles each rounding step)
            match unary_int(|m, pow| m.checked_neg()?.div_euclid(pow).checked_neg())? {
                NumVal::Dec(m, _) => NumVal::Int(m),
                v => v,
            }
        }
        Func::Exponentiation => {
            // base^exp exactly for an integer exponent ≥ 0 (cwm: 2.7² = 7.29).
            if vals.len() != 2 {
                return None;
            }
            let (NumVal::Int(e), base) = (vals[1], vals[0]) else { return None };
            if e < 0 {
                return None; // a negative exponent: EYE's double result (f64 path)
            }
            let (m, sc) = match base {
                NumVal::Int(i) => (i, 0u32),
                NumVal::Dec(m, sc) => (m, sc),
                NumVal::F64(_) => return None,
            };
            if e > i128::from(u32::MAX) {
                rep::<()>(pending, None)?;
            }
            let e = e as u32;
            let acc = rep(pending, m.checked_pow(e))?;
            let scale = rep(pending, sc.checked_mul(e))?;
            // A decimal result past 34 fraction digits is past the tower's scale: a cut.
            rep(pending, (scale <= 34).then_some(()))?;
            if any_dec {
                NumVal::Dec(acc, scale)
            } else {
                NumVal::Int(acc)
            }
        }
        _ => return None, // trig/log: f64 path
    };
    Some(out)
}

/// Extract a component of an `xsd:dateTime`/`xsd:date` lexical form for `time:` builtins.
/// Lexical: `[-]YYYY-MM-DD[Thh:mm:ss[.sss]][Z|±hh:mm]`.
fn datetime_part(s: &str, f: Func, pending: &bounded::Pending) -> Option<i64> {
    let (date, time) = s.split_once('T').unwrap_or((s, ""));
    let neg = date.starts_with('-');
    let mut dparts = date.trim_start_matches('-').split('-');
    match f {
        Func::Year => {
            settle(pending, bounded::digits_i64(dparts.next()?)).map(|y| if neg { -y } else { y })
        }
        Func::Month => settle(pending, bounded::digits_i64(dparts.nth(1)?)),
        Func::Day => settle(pending, bounded::digits_i64(dparts.nth(2)?)),
        Func::Hours | Func::Minutes | Func::Seconds => {
            // strip any timezone (Z, +hh:mm, -hh:mm) — the time itself has no +/-/Z.
            let t = time.split(['+', '-', 'Z']).next().unwrap_or(time);
            let idx = match f {
                Func::Hours => 0,
                Func::Minutes => 1,
                Func::Seconds => 2,
                _ => unreachable!(),
            };
            let part = t.split(':').nth(idx)?;
            settle(
                pending,
                bounded::digits_i64(part.split('.').next().unwrap_or(part)),
            )
        }
        Func::DayOfWeek | Func::InSeconds => {
            let (days, secs, fraction) = epoch_parts(s, pending)?;
            if matches!(f, Func::DayOfWeek) {
                return Some((days + 4).rem_euclid(7)); // 1970-01-01 = Thursday
            }
            // A whole second count: a nonzero fraction it drops is lost precision.
            settle(pending, bounded::dropped_fraction(fraction));
            settle(
                pending,
                bounded::exact(days.checked_mul(86400).and_then(|d| d.checked_add(secs))),
            )
        }
        _ => None,
    }
}

/// Split a `[-]YYYY[-MM[-DD]][Thh[:mm[:ss[.sss]]]][Z|±hh:mm]` lexical form into
/// (days since 1970-01-01, seconds-of-day shifted to UTC) for `time:dayOfWeek` /
/// `time:inSeconds`. Only genuinely ABSENT components take the cwm defaults
/// ("2002" = 2002-01-01T00:00:00Z); a SUPPLIED field that does not parse, is out
/// of range, or names an impossible calendar date declines (`None`) — it must never
/// fall back to a default and so bind a different, valid instant. The timezone
/// suffix is separated first (on a date as well as a dateTime) so it is never read
/// as part of a calendar or clock field. #3804.
fn epoch_parts<'s>(s: &'s str, pending: &bounded::Pending) -> Option<(i64, i64, &'s str)> {
    let field = |x: &str, range: std::ops::RangeInclusive<i64>| -> Option<i64> {
        let v = settle(pending, bounded::digits_i64(x))?;
        range.contains(&v).then_some(v)
    };
    // Timezone suffix: `Z`, or `±hh:mm` at the very end (the `-` of a date body is
    // followed by more than `hh:mm`, so the fixed-width tail is unambiguous).
    let (body, offset) = if let Some(b) = s.strip_suffix('Z') {
        (b, 0)
    } else if s.len() >= 6
        && s.is_char_boundary(s.len() - 6)
        && matches!(s.as_bytes()[s.len() - 6], b'+' | b'-')
        && s.as_bytes()[s.len() - 3] == b':'
    {
        let (b, tz) = s.split_at(s.len() - 6);
        let sign = if tz.starts_with('-') { -1 } else { 1 };
        let oh = field(&tz[1..3], 0..=14)?;
        let om = field(&tz[4..6], 0..=59)?;
        (b, sign * (oh * 3600 + om * 60))
    } else {
        (s, 0)
    };
    let (date, time) = match body.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (body, None),
    };
    let (neg, date) = match date.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, date),
    };
    let mut dp = date.split('-');
    // The year is checked on its digits in bounded.rs (a year past the range, even past
    // i64, is a cut; malformed text is not).
    let y = settle(pending, bounded::epoch_year(dp.next()?))?;
    let y = if neg { -y } else { y };
    let m = dp.next().map(|x| field(x, 1..=12)).unwrap_or(Some(1))?;
    let d = dp.next().map(|x| field(x, 1..=31)).unwrap_or(Some(1))?;
    if dp.next().is_some() || d > days_in_month(y, m)? {
        return None;
    }
    let mut tod = 0i64;
    // The seconds field's fraction, which whole-second results drop.
    let mut dropped = "";
    if let Some(t) = time {
        let mut tp = t.split(':');
        let hh = field(tp.next()?, 0..=24)?;
        let mi = tp.next().map(|x| field(x, 0..=59)).unwrap_or(Some(0))?;
        let ss = match tp.next() {
            None => 0,
            Some(x) => {
                let (whole, frac) = x.split_once('.').unwrap_or((x, "0"));
                if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                dropped = frac;
                field(whole, 0..=59)?
            }
        };
        if tp.next().is_some() || (hh == 24 && (mi, ss) != (0, 0)) {
            return None;
        }
        tod = hh * 3600 + mi * 60 + ss;
    }
    Some((days_from_civil(y, m, d), tod - offset, dropped))
}

/// Number of days in month `m` (1-12) of proleptic-Gregorian year `y`; `None` for an
/// out-of-range month. #3804.
fn days_in_month(y: i64, m: i64) -> Option<i64> {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    Some(match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    })
}

/// Days since 1970-01-01 of the civil date y-m-d (Howard Hinnant's algorithm).
/// Callers validate the date first (see [`days_in_month`]); the formula itself
/// silently rolls an impossible date over.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Epoch seconds → `YYYY-MM-DDThh:mm:ssZ` (time:inSeconds reverse mode).
fn format_epoch(secs: i64) -> String {
    let mut s = String::new();
    // Writing to a `String` cannot fail.
    let _ = write_epoch(secs, &mut s);
    s
}

/// The length of [`format_epoch`]'s text, counted without building it.
fn epoch_len(secs: i64) -> usize {
    let mut n = LexLen::default();
    // Writing to a counter cannot fail.
    let _ = write_epoch(secs, &mut n);
    n.len
}

/// Writes [`format_epoch`]'s text to `out`.
fn write_epoch(secs: i64, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // civil_from_days (Hinnant)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    write!(
        out,
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Render an `f64` as a cwm-style `xsd:double` literal: `INF`/`-INF`/`NaN`, or
/// e-notation with a fractional digit in the mantissa (`0.0e0`, `7.29e0`).
fn double_term(v: f64) -> Term {
    const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
    let lex = if v.is_nan() {
        "NaN".to_string()
    } else if v == f64::INFINITY {
        "INF".to_string()
    } else if v == f64::NEG_INFINITY {
        "-INF".to_string()
    } else {
        let s = format!("{v:e}"); // e.g. "0e0", "7.29e0", "1.23e3"
        match s.split_once('e') {
            Some((m, e)) if !m.contains('.') => format!("{m}.0e{e}"),
            _ => s,
        }
    };
    Term::Lit(lex, XSD_DOUBLE.into(), None)
}

/// Render an `f64` result as an N3 numeric literal (integer when whole, else decimal).
/// A non-finite value renders as `inf` / `-inf` / `NaN`^^xsd:decimal, which `cmp_num`
/// reads back as ±INF / NaN.
fn number_term(v: f64) -> Term {
    if number_is_whole(v) {
        // not-a-cut: exact-cast (whole, below 2^53)
        Term::Lit((v as i64).to_string(), "http://www.w3.org/2001/XMLSchema#integer".into(), None)
    } else {
        Term::Lit(format!("{v}"), "http://www.w3.org/2001/XMLSchema#decimal".into(), None)
    }
}

/// Intern an N3 ground term into the dictionary.
fn intern(dict: &mut Dict, t: &Term) -> Result<Id, String> {
    Ok(match t {
        Term::Iri(i) => dict.intern_iri(i),
        Term::Lit(v, dt, lang) => dict.intern_lit(v, dt, lang.as_deref()),
        Term::Blank(b) => dict.intern_blank(b),
        // A ground quoted triple interns through `Dict`'s RDF 1.2 triple-term
        // path (content-addressed by component ids), via the same `oxrdf`
        // representation the Turtle/N-Triples loaders use — so an N3-derived
        // `<< s p o >>` and a store-loaded `<<( s p o )>>` share one id. [FABLE-5]
        Term::Triple(_) => dict.intern(&n3_term_to_oxrdf(t)?),
        Term::Var(_) | Term::Formula(_) => return Err("non-ground term in closure".into()),
        Term::List(_) => return Err("unexpanded list term in closure".into()),
    })
}

/// Convert a GROUND N3 term into its `oxrdf` form for dictionary interning of
/// quoted-triple components. Mirrors the acceptance of the component interners
/// exactly (`intern_iri`/`intern_blank` validate nothing, so `new_unchecked` —
/// the SAME strings intern identically inside and outside a quoted triple).
/// RDF 1.2 structural constraints apply: a triple term's subject must be an
/// IRI or blank node and its predicate an IRI — anything else (incl. N3's
/// generalized literal-subject triples, formulae, unexpanded lists) is a loud
/// error, never a silent re-encoding. [FABLE-5]
fn n3_term_to_oxrdf(t: &Term) -> Result<oxrdf::Term, String> {
    Ok(match t {
        Term::Iri(i) => oxrdf::NamedNode::new_unchecked(i).into(),
        Term::Lit(v, dt, lang) => match lang {
            Some(l) => oxrdf::Literal::new_language_tagged_literal_unchecked(v, l).into(),
            None => {
                oxrdf::Literal::new_typed_literal(v, oxrdf::NamedNode::new_unchecked(dt)).into()
            }
        },
        Term::Blank(b) => oxrdf::BlankNode::new_unchecked(b).into(),
        Term::Triple(tr) => {
            let s: oxrdf::NamedOrBlankNode = match &tr[0] {
                Term::Iri(i) => oxrdf::NamedNode::new_unchecked(i).into(),
                Term::Blank(b) => oxrdf::BlankNode::new_unchecked(b).into(),
                other => {
                    return Err(format!(
                        "quoted-triple subject {} is not an IRI or blank node (RDF 1.2 triple terms admit no other subject kind)",
                        serialize::display_lossy(other)
                    ))
                }
            };
            let Term::Iri(p) = &tr[1] else {
                return Err(format!(
                    "quoted-triple predicate {} is not an IRI (RDF 1.2 triple terms admit no other predicate kind)",
                    serialize::display_lossy(&tr[1])
                ));
            };
            let o = n3_term_to_oxrdf(&tr[2])?;
            oxrdf::Term::Triple(Box::new(oxrdf::Triple::new(
                s,
                oxrdf::NamedNode::new_unchecked(p),
                o,
            )))
        }
        Term::Var(_) | Term::Formula(_) | Term::List(_) => {
            return Err(format!("term {} inside a quoted triple has no dictionary representation", serialize::display_lossy(t)))
        }
    })
}

#[cfg(test)]
mod runtime_doc_key_golden {
    use super::run_shared::runtime_doc_key;

    /// The runtime-document key is specified, not build-dependent: these vectors were
    /// computed independently (SHA-256 over the documented encoding, first 8 bytes). A
    /// change here changes exported labels and provenance IRIs.
    #[test]
    fn runtime_doc_key_matches_golden_vectors() {
        let x = "_:x <http://ex/p> <http://ex/o> .";
        for (text, base, key) in [
            ("", "", "80ab39c5ca307a54"),
            (x, "", "36e2d3c73471b49f"),
            (x, "http://ex/doc", "35978aa649d31637"),
            ("ab", "c", "f053a10b92eecfd0"),
            ("a", "bc", "ff5cfd007e1caf6c"),
        ] {
            assert_eq!(runtime_doc_key(text, base), key, "{text:?} {base:?}");
        }
    }

    #[test]
    fn runtime_label_shape() {
        use super::run_shared::is_runtime_label;
        for l in ["__rp0_36e2d3c73471b49f_x", "__rp12_0123456789abcdef_a.b", "__rp0_36e2d3c73471b49f__"] {
            assert!(is_runtime_label(l), "{l}");
        }
        for l in [
            "__rp0_x",
            "__rp_36e2d3c73471b49f_x",
            "__rp0_36E2D3C73471B49F_x",
            "__rp0_36e2d3c73471b49_x",
            "__rp0_36e2d3c73471b49fx",
            "__rp0_36e2d3c73471b49f",
            "_rp0_36e2d3c73471b49f_x",
        ] {
            assert!(!is_runtime_label(l), "{l}");
        }
    }
}

#[cfg(test)]
mod tests {
    /// GH #6901: the length of a numeral's lexical form, computed from its value, equals
    /// the length of the literal it renders, for every rendering (integer, decimal at any
    /// scale, whole and fractional `f64`, `xsd:double` e-notation, non-finite values), and
    /// the cheap bound is at least it.
    #[test]
    fn numeral_lengths_match_the_rendered_literals() {
        use super::{bounded, NumVal, Numeral, Term};
        let mut cases: Vec<Numeral> = Vec::new();
        let ints = [
            0i128,
            1,
            -1,
            9,
            10,
            -10,
            99,
            100,
            12345,
            -98765,
            i128::from(i64::MAX),
            i128::from(i64::MIN),
            10i128.pow(18),
            10i128.pow(18) - 1,
            10i128.pow(38),
            10i128.pow(38) - 1,
            i128::MAX,
            i128::MIN,
            i128::MIN + 1,
        ];
        for &i in &ints {
            cases.push(Numeral::Num(NumVal::Int(i)));
            for s in [0u32, 1, 2, 3, 5, 17, 18, 19, 37, 38, 39, 40, 41, 64, 200, 1000] {
                cases.push(Numeral::Num(NumVal::Dec(i, s)));
                cases.push(Numeral::Num(NumVal::Dec(i.saturating_mul(10), s)));
                cases.push(Numeral::Num(NumVal::Dec(i.saturating_mul(1000), s)));
            }
        }
        let floats = [
            0.0f64,
            -0.0,
            1.0,
            -1.0,
            0.5,
            -0.25,
            7.29,
            1.0 / 3.0,
            -2.0 / 3.0,
            123456.789,
            9.006e15,
            9.007e15,
            -9.007e15,
            1e16,
            1e21,
            1e22,
            1e100,
            1e300,
            -1e300,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            5e-324,
            f64::from_bits(0x000f_ffff_ffff_ffff),
            -f64::from_bits(0x0008_1234_5678_9abc),
            f64::from_bits(0x7fef_1234_5678_9abc),
            1e-7,
            1.5e-5,
            std::f64::consts::PI,
            std::f64::consts::E * 1e10,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        for &f in &floats {
            cases.push(Numeral::Num(NumVal::F64(f)));
            cases.push(Numeral::Double(f));
        }
        for n in cases {
            let t = n.term();
            let Term::Lit(lex, dt, None) = &t else { panic!("{n:?}: not a plain literal: {t:?}") };
            assert_eq!(n.lex_len(), lex.len(), "{n:?}: computed length vs rendered {lex:?}");
            assert_eq!(n.datatype(), dt.as_str(), "{n:?}: datatype");
            assert_eq!(n.term_bytes(), bounded::term_bytes(&t), "{n:?}: term bytes");
            assert!(n.term_bound() >= n.term_bytes(), "{n:?}: bound below the exact size");
        }
        // A decimal's length follows its scale without rendering it.
        let long = Numeral::Num(NumVal::Dec(1, u32::MAX));
        assert_eq!(long.lex_len(), u32::MAX as usize + 2);
        assert!(long.term_bound() >= long.term_bytes());
        assert_eq!(Numeral::Num(NumVal::Dec(0, u32::MAX)).lex_len(), "0.0".len());
    }

    /// GH #6901: `string:concatenation`'s numeric pieces are sized from their values, and
    /// the size is the length of the text the builtin writes.
    #[test]
    fn concat_piece_lengths_match_the_pieces() {
        use super::{concat_piece, concat_piece_len, Term};
        const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
        let big = format!("-000{}.{}000", "9".repeat(60), "1".repeat(50));
        let tiny = format!("0.{}1", "0".repeat(400));
        let lits: Vec<(String, &str)> = vec![
            ("0".into(), "boolean"),
            ("true".into(), "boolean"),
            ("false".into(), "boolean"),
            ("1".into(), "boolean"),
            ("42".into(), "integer"),
            ("-0042".into(), "integer"),
            ("+7".into(), "integer"),
            (" 3 ".into(), "integer"),
            ("170141183460469231731687303715884105727".into(), "integer"),
            ("-170141183460469231731687303715884105728".into(), "integer"),
            ("170141183460469231731687303715884105728".into(), "integer"),
            ("1.50".into(), "decimal"),
            ("-0.0".into(), "decimal"),
            ("30.0".into(), "decimal"),
            (".5".into(), "decimal"),
            ("-.5".into(), "decimal"),
            ("5.".into(), "decimal"),
            (big, "decimal"),
            (tiny, "decimal"),
            ("1E300".into(), "double"),
            ("0E1".into(), "double"),
            ("1.5e-7".into(), "double"),
            ("-2.5E3".into(), "float"),
            ("INF".into(), "double"),
            ("-INF".into(), "double"),
            ("NaN".into(), "double"),
            ("abc".into(), "double"),
            ("x".into(), "string"),
            ("é".into(), "string"),
        ];
        let mut terms: Vec<Term> = lits
            .into_iter()
            .map(|(v, local)| Term::Lit(v, format!("{XSD}{local}"), None))
            .collect();
        terms.push(Term::Iri("http://ex/i".into()));
        terms.push(Term::Blank("b".into()));
        terms.push(Term::List(vec![]));
        for t in &terms {
            assert_eq!(concat_piece_len(t), concat_piece(t).map(|p| p.len()), "{t:?}");
        }
    }

    /// GH #6901: a decimal's digits are read without joining them into a new string, with
    /// exactly the result of parsing the joined text.
    #[test]
    fn joined_digits_read_as_the_joined_text_parses() {
        let parts = [
            ("", ""),
            ("", "5"),
            ("1", ""),
            ("12", "34"),
            ("-1", "5"),
            ("+1", "5"),
            ("-", "5"),
            ("+", ""),
            ("-", ""),
            ("", "-5"),
            ("1-", "5"),
            ("--1", "5"),
            ("+-1", "5"),
            ("1_0", "5"),
            (" 1", "5"),
            ("0000000000000000000000000000000000000000000000", "0001"),
            ("17014118346046923173168730371588410572", "7"),
            ("17014118346046923173168730371588410572", "8"),
            ("-17014118346046923173168730371588410572", "8"),
            ("-17014118346046923173168730371588410572", "9"),
            ("99999999999999999999999999999999999999999", "9"),
            ("١", "2"),
        ];
        for (int, frac) in parts {
            let want = format!("{int}{frac}").parse::<i128>().ok();
            assert_eq!(super::joined_i128(int, frac), want, "{int:?} {frac:?}");
        }
    }

    /// GH #6901: `time:inSeconds` reversed is sized from the epoch without formatting it.
    #[test]
    fn epoch_lengths_match_the_formatted_text() {
        for secs in [0i64, 1, -1, 86_399, 86_400, 1_700_000_000, -62_135_596_800, -62_167_219_200, -1_000_000_000_000, 1_000_000_000_000, i64::MAX / 2, i64::MIN / 2] {
            assert_eq!(super::epoch_len(secs), super::format_epoch(secs).len(), "{secs}");
        }
    }

    /// GH #6771 review: the replacement size is counted, not built, and the count is what
    /// `replace_all` writes, under every corner of the replacement syntax.
    #[test]
    fn replace_bytes_counts_what_replace_all_writes() {
        let cases: &[(&str, &str, &str)] = &[
            ("banana", "a", "[$0]"),
            ("banana", "(a)(n)?", "$2$1$$"),
            ("banana", "(?P<v>a)", "${v}-$v-${nope}-$nope"),
            ("banana", "a", "$1a${1}b"),
            ("banana", "a", "$"),
            ("banana", "a", "x$"),
            ("banana", "a", "${unclosed"),
            ("banana", "a", "$-$ $."),
            ("banana", "", "<>"),
            ("ünïcödé", "[öé]", "$0$0"),
            ("", "x*", "y"),
        ];
        for &(s, pattern, r) in cases {
            let re = regex::Regex::new(pattern).expect("pattern");
            let want = re.replace_all(s, r).len();
            assert_eq!(super::replace_bytes(&re, s, r), want, "{s:?} {pattern:?} {r:?}");
        }
    }

    /// GH #6771 review: for each functional builtin whose result `output_bytes` sizes (the
    /// non-numeric ones; a numeric result is sized from its value, see
    /// `numeral_lengths_match_the_rendered_literals`), the exact size equals the
    /// [`bounded::term_bytes`] of the term the builtin actually derives, and the cheap bound
    /// is at least it.
    #[test]
    fn each_sized_result_matches_the_term_the_builtin_derives() {
        use super::{bounded, functional_builtin, output_bytes, FactIndex, Term};
        const STRING: &str = "http://www.w3.org/2000/10/swap/string#";
        const LIST: &str = "http://www.w3.org/2000/10/swap/list#";
        const LOG: &str = "http://www.w3.org/2000/10/swap/log#";
        const TIME: &str = "http://www.w3.org/2000/10/swap/time#";
        // (builtin, subject): each subject is a first-class list or a single value.
        let cases: &[(&str, &str, &str)] = &[
            (STRING, "concatenation", r#"("aÉ" "b" <http://ex/i> true)"#),
            (STRING, "format", r#"("%s and %s %%" "x" "yé")"#),
            (STRING, "replace", r#"("banana" "(an)" "<$1$1>")"#),
            (STRING, "lowerCase", r#""AÉ /#ẞ""#),
            (STRING, "upperCase", r#""aé /#ß""#),
            (STRING, "encodeForUri", r#""a É/#~(x)""#),
            (STRING, "encodeForURI", r#""a É/#~(x)""#),
            (STRING, "encodeForFragID", r#""a É/#~(x)""#),
            (STRING, "scrape", r#"("key=value; rest" "=([a-z]+);")"#),
            (LIST, "append", r#"(("a" "b") ("c") ())"#),
            (LIST, "first", r#"("first" "second")"#),
            (LIST, "last", r#"("first" "second")"#),
            (LOG, "conjunction", r#"({ :s :p "a" } { :s :p "a". :s :q "b" })"#),
            // GH #6901: no triples, so the result is the literal `true`, not a formula.
            (LOG, "conjunction", "(true)"),
            (LOG, "conjunction", "({} true {})"),
            (LOG, "conjunction", "()"),
            (STRING, "concatenation", r#"(1.50 -0042 "1E300"^^<http://www.w3.org/2001/XMLSchema#double> 0.000001 "x")"#),
            (LOG, "langlit", r#"("chat" "fr")"#),
            (TIME, "timeZone", r#""2024-03-15T10:30:45+05:30""#),
            (TIME, "timeZone", r#""2024-03-15T10:30:45-11:00""#),
        ];
        for &(ns, name, subject) in cases {
            let iri = format!("{ns}{name}");
            let src = format!(
                "@prefix : <http://ex/> .\n:in :v {subject} .\n\
                 {{ :in :v ?s . ?s <{iri}> ?o }} => {{ :r :out ?o }} .\n"
            );
            let c = super::reason_n3_terms(&src, None).expect(name);
            let input = &c.facts.iter().find(|t| t[1] == Term::Iri("http://ex/v".into())).expect("input")[2];
            let args: Vec<Term> = match input {
                Term::List(ms) => ms.clone(),
                other => vec![other.clone()],
            };
            let out = c
                .derived
                .iter()
                .find(|t| t[1] == Term::Iri("http://ex/out".into()))
                .unwrap_or_else(|| panic!("{name}: no result"));
            let actual = bounded::term_bytes(&out[2]);
            let func = functional_builtin(&Term::Iri(iri.clone())).expect(name);
            let (facts, pending) = (FactIndex::default(), bounded::Pending::default());
            let bound = output_bytes(func, &args, &facts, &pending, false);
            let exact = output_bytes(func, &args, &facts, &pending, true);
            let _ = pending.finish(());
            assert_eq!(exact, actual, "{name} {subject}: exact size vs the derived {:?}", out[2]);
            assert!(bound >= actual, "{name} {subject}: bound {bound} < derived {actual}");
        }
    }

    /// #6732: a document nested to the parser's limit is reasoned over on a 2 MiB stack
    /// without overflowing: matching, instantiation, interning, serialization, proofs,
    /// queries, and a `log:parsedAsN3` of the same document inside a rule.
    #[test]
    fn a_document_nested_to_the_parse_limit_reasons_on_a_small_stack() {
        use super::parser::parse_surface_tests::{nested, on_small_stack, PRODUCTIONS};
        let limit = super::bounded::parse_depth();
        let rules = "{ ?s :p ?o } => { ?s :q ?o } .\n\
            { :s :q ?o . :s :p ?o2 } => { :w :same ?o2 } .\n";
        for production in PRODUCTIONS {
            let results = on_small_stack(move || {
                let doc = format!("{}{rules}", nested(production, limit));
                let escaped = nested(production, limit)
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n");
                let parsed_in_rule = format!(
                    "@prefix : <http://ex/> .\n\
                     @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
                     {{ \"{escaped}\" log:parsedAsN3 ?f . ?f log:includes {{ :s :p ?o }} }} \
                     => {{ :r :got ?o }} .\n"
                );
                let derives = |src: &str, subject: &str| {
                    super::reason_n3_terms(src, None).map(|c| {
                        let subject = super::Term::Iri(format!("http://ex/{subject}"));
                        c.derived.iter().any(|t| t[0] == subject)
                    })
                };
                let terms = derives(&doc, "w");
                let in_rule = derives(&parsed_in_rule, "r");
                // These may refuse a term shape (e.g. a formula has no dictionary id);
                // what matters is that they return.
                let _ = super::reason_n3(&mut sparq_core::dict::Dict::new(), &doc);
                let _ = super::reason_n3_proof(&mut sparq_core::dict::Dict::new(), &doc);
                let _ = super::reason_n3_pass_all(&doc, super::RuleVars::N3);
                let _ = super::reason_n3_query_terms(&doc, "{ :s :q ?o } => { :s :q ?o } .");
                (terms, in_rule)
            });
            assert_eq!(results, (Ok(true), Ok(true)), "{production}");
        }
    }

    /// The closure of `src` (with `resolver`) on a fresh run: its facts or error, and
    /// the run's cut.
    fn closure_and_cut(
        src: &str,
        resolver: Option<&Resolver>,
    ) -> (Result<Vec<[Term; 3]>, String>, Option<&'static str>) {
        let run = Truncation::top_level();
        let r = reason_n3_terms_in(src, None, resolver, NegationCycles::Reject, &run);
        (r.map(|c| c.facts), run.get())
    }

    /// #6953: a `{ … }` pattern matched against a `{ … }` value backtracks over statement
    /// pairings. A pattern whose last statement matches nothing tries every ordering of
    /// the others first; its budget stops it, as a cut, and the rule does not fire. A
    /// negation in the same run is then refused.
    #[test]
    fn formula_unification_backtracking_is_budgeted() {
        let n = 10;
        let value: String = (0..n).map(|i| format!(":s :p :o{i} . ")).collect();
        let pattern: String = (0..n).map(|i| format!(":s :p ?v{i} . ")).collect();
        let doc = |negation: &str| {
            format!(
                "@prefix : <http://ex/> .\n\
                 @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
                 :d :f {{ {value}:t :r :x }} .\n\
                 {{ :d :f {{ {pattern}:t :r :y }} }} => {{ :m :m :m }} .\n\
                 {negation}"
            )
        };
        let (facts, cut) = closure_and_cut(&doc(""), None);
        let facts = facts.expect("a cut alone does not refuse a positive run");
        assert!(!facts.iter().any(|t| t[0] == Term::Iri("http://ex/m".into())));
        assert_eq!(cut, Some("formula unification exhausted its step budget"));

        let negation = "{ ?x log:notIncludes { :m :m :m } } => { :n :n :n } .";
        let (refused, _) = closure_and_cut(&doc(negation), None);
        assert!(refused.is_err(), "{refused:?}");

        // The same match inside a formula-containment search stops that search, as a cut.
        let containment = format!(
            "@prefix : <http://ex/> .\n\
             @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             {{ {{ :d :f {{ {value}:t :r :x }} }} log:includes \
             {{ :d :f {{ {pattern}:t :r :y }} }} }} => {{ :m :m :m }} ."
        );
        let (facts, cut) = closure_and_cut(&containment, None);
        assert!(!facts.unwrap().iter().any(|t| t[0] == Term::Iri("http://ex/m".into())));
        assert_eq!(cut, Some("formula containment search exhausted its step budget"));
    }

    /// #6953: a match that never backtracks is never refused, however many statements
    /// it pairs (here far more pairings than the backtracking budget), and a match that
    /// backtracks a little still succeeds.
    #[test]
    fn formula_unification_without_backtracking_is_not_budgeted() {
        // The pattern lists the value's statements in reverse, so pairing each one scans
        // past every unused value statement before it: n * (n + 1) / 2 pairings.
        let n = 1000;
        let value: String = (0..n).map(|i| format!(":s :p :o{i} . ")).collect();
        let reversed: String = (0..n).rev().map(|i| format!(":s :p :o{i} . ")).collect();
        let doc = format!(
            "@prefix : <http://ex/> .\n\
             :d :f {{ {value} }} .\n\
             :e :f {{ :a :p :c . :a :p :b . :c :q :z }} .\n\
             {{ :d :f {{ {reversed} }} }} => {{ :m :m :m }} .\n\
             {{ :e :f {{ :a :p ?x . :a :p ?y . ?y :q :z }} }} => {{ :k :k ?x }} ."
        );
        let (facts, cut) = closure_and_cut(&doc, None);
        let facts = facts.unwrap();
        let m = Term::Iri("http://ex/m".into());
        assert!(facts.contains(&[m.clone(), m.clone(), m]));
        let k = Term::Iri("http://ex/k".into());
        assert!(facts.contains(&[k.clone(), k, Term::Iri("http://ex/b".into())]));
        assert_eq!(cut, None);
    }

    /// The deepest nesting of `t`, counted in list, formula and quoted-triple levels.
    fn depth_of(t: &Term) -> usize {
        match t {
            Term::List(ms) => 1 + ms.iter().map(depth_of).max().unwrap_or(0),
            Term::Formula(ts) => 1 + ts.iter().flatten().map(depth_of).max().unwrap_or(0),
            Term::Triple(tr) => 1 + tr.iter().map(depth_of).max().unwrap_or(0),
            _ => 0,
        }
    }

    /// #6732: a rule that wraps a term in one more list each round deepens it without
    /// end, and a chain of premise atoms deepens it within one firing. A generated term
    /// past the term-depth limit is a cut and is dropped, so the closure ends (a negation
    /// over it is refused), on a 2 MiB stack, and every entry point walks what it kept.
    #[test]
    fn rule_generated_nesting_stops_at_the_term_depth_limit_on_a_small_stack() {
        use super::parser::parse_surface_tests::on_small_stack;
        let limit = super::bounded::term_depth();
        let pre = "@prefix : <http://ex/> .\n\
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";
        let round = format!("{pre}:s :p :z .\n{{ :s :p ?x }} => {{ :s :p (?x) }} .\n");
        let wrap = 60;
        let wide_wrap = format!(
            "{pre}:s :p :z .\n{{ :s :p ?x }} => {{ :s :p {}?x{} }} .\n",
            "(".repeat(wrap),
            ")".repeat(wrap)
        );
        let chain: String = (0..5_000)
            .map(|i| format!("((?y{i})) rdf:first ?y{} . ", i + 1))
            .collect();
        let firing = format!("{pre}:s :p :z .\n{{ :s :p ?y0 . {chain}}} => {{ :s :q ?y5000 }} .\n");
        let negated = format!(
            "{round}{{ :s :p ?x . ?scope log:notIncludes {{ :s :never ?x }} }} => {{ :s :ok ?x }} .\n"
        );
        let results = on_small_stack(move || {
            let round_trip = closure_and_cut(&round, None);
            let wide = closure_and_cut(&wide_wrap, None);
            let firing = closure_and_cut(&firing, None);
            let negated = closure_and_cut(&negated, None);
            // The other entry points walk every kept term (interning, proofs,
            // serialization, a query's projection) on the same stack.
            let _ = super::reason_n3(&mut sparq_core::dict::Dict::new(), &round);
            let _ = super::reason_n3_proof(&mut sparq_core::dict::Dict::new(), &round);
            let _ = super::reason_n3_pass_all(&round, super::RuleVars::N3);
            let _ = super::reason_n3_query_terms(&round, "{ :s :p ?o } => { :s :p ?o } .");
            (round_trip, wide, firing, negated)
        });
        let (round_trip, wide, firing, negated) = results;
        // One fact per depth up to the limit, each kept term within it, and a cut.
        let (facts, cut) = round_trip;
        let facts = facts.expect("the closure ends at the limit");
        assert_eq!(facts.len(), limit + 1);
        assert_eq!(facts.iter().flatten().map(depth_of).max(), Some(limit));
        assert!(cut.is_some());
        let (facts, cut) = wide;
        let facts = facts.expect("the closure ends at the limit");
        assert_eq!(facts.len(), limit / wrap + 1);
        assert!(cut.is_some());
        let (facts, cut) = firing;
        let facts = facts.expect("the firing is dropped");
        assert_eq!(facts.len(), 1);
        assert!(cut.is_some());
        let (refused, cut) = negated;
        let e = refused.expect_err("a negation over the cut closure is refused");
        assert!(e.contains("refused a negation"), "{e}");
        assert!(cut.is_some());
    }

    /// #6732: a chain of distinct documents, each closing the next one's
    /// `log:semantics` with `log:conclusion`, nests closures without bound, and the
    /// import-cycle guard does not see it. Each level here closes the next from inside a
    /// backward proof `steps` deep, after `scopes` nested store-scoped negations, next to
    /// a fact nested 60 deep. Every level draws on the run's one stack allowance: on a
    /// 2 MiB stack an endless chain is refused (or, when proof steps or scopes spend the
    /// allowance first, cut), never a stack overflow; a short chain closes.
    #[test]
    fn nested_closures_share_the_runs_stack_allowance_on_a_small_stack() {
        use super::parser::parse_surface_tests::on_small_stack;
        let doc = |i: usize, last: usize, steps: usize, scopes: usize| -> String {
            if i == last {
                return "<http://ex/end> <http://ex/is> <http://ex/reached> .".into();
            }
            let mut d = String::from(
                "@prefix : <http://ex/> .\n\
                 @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n",
            );
            d.push_str(&format!(":d :e {}:z{} .\n", "(".repeat(60), ")".repeat(60)));
            for j in 0..steps {
                d.push_str(&format!("{{ :a :p{j} :b }} <= {{ :a :p{} :b }} .\n", j + 1));
            }
            // An odd number of nested negations of a statement never asserted holds.
            let negated = (0..scopes).fold(":q :never :q".to_string(), |inner, _| {
                format!("?x log:notIncludes {{ {inner} }}")
            });
            let negated = if scopes == 0 {
                String::new()
            } else {
                format!("{negated} . ")
            };
            d.push_str(&format!(
                "{{ :a :p{steps} :b }} <= {{ {negated}<http://doc/{}> log:semantics ?f . \
                 ?f log:conclusion ?c }} .\n",
                i + 1
            ));
            d.push_str(&format!("{{ :a :p0 :b }} => {{ :s :p :o{i} }} .\n"));
            d
        };
        // Whether the first document derived its fact, or the error; and the run's cut.
        let run = move |last: usize, steps: usize, scopes: usize| {
            on_small_stack(move || {
                let resolver = move |iri: &str| {
                    let n: usize = iri.strip_prefix("http://doc/")?.parse().ok()?;
                    Some(doc(n, last, steps, scopes))
                };
                let top = doc(0, last, steps, scopes);
                let (facts, cut) = closure_and_cut(&top, Some(&resolver));
                let o0 = Term::Iri("http://ex/o0".into());
                (facts.map(|f| f.iter().any(|t| t[2] == o0)), cut)
            })
        };
        assert_eq!(run(3, 20, 0), (Ok(true), None), "a short chain closes");
        assert_eq!(run(3, 0, 3), (Ok(true), None), "a short chain closes");
        // Closures alone: refused when the allowance is spent.
        let (refused, cut) = run(usize::MAX, 0, 0);
        let e = refused.expect_err("an endless chain is refused");
        assert!(e.contains("nested closures passed"), "{e}");
        assert!(cut.is_some());
        // Deep backward proofs at every level: the allowance is spent by proof steps,
        // which is a cut (the levels above still close, over what was proved).
        for (steps, scopes) in [(20, 0), (60, 0), (0, 31), (30, 31)] {
            let (_, cut) = run(usize::MAX, steps, scopes);
            assert!(cut.is_some(), "{steps} {scopes}");
        }
    }

    /// #6732: matching a formula of many statements against another, or checking that
    /// one includes them, takes no stack per statement: thousands of statements match
    /// on a 2 MiB stack.
    #[test]
    fn wide_formulae_match_on_a_small_stack() {
        use super::parser::parse_surface_tests::on_small_stack;
        let n = 20_000;
        let body = ":a :b :c . ".repeat(n);
        let rest = ":a :b :c . ".repeat(n - 1);
        let src = format!(
            "@prefix : <http://ex/> .\n\
             @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             :s :p {{ {body} }} .\n\
             :s :q {{ :a :b :c }} .\n\
             {{ :s :p {{ :a :b ?v . {rest} }} }} => {{ :done :unified ?v }} .\n\
             {{ :s :q ?f . ?f log:includes {{ {body} }} }} => {{ :done :included :true }} .\n"
        );
        let (facts, cut) = on_small_stack(move || closure_and_cut(&src, None));
        let facts = facts.expect("closes");
        let done = Term::Iri("http://ex/done".into());
        let derived: Vec<&[Term; 3]> = facts.iter().filter(|t| t[0] == done).collect();
        assert_eq!(derived.len(), 2, "{derived:?}");
        assert_eq!(cut, None);
    }

    /// #6732: the containment search of a formula scope (`log:includes` over a quoted
    /// formula) checks every binding it extends against the term-depth limit before
    /// carrying it on, including the bindings virtual list access builds. A pattern
    /// whose bindings would pass the limit is dropped as a cut, on a 2 MiB stack, and
    /// the run ends.
    #[test]
    fn formula_containment_bindings_stay_within_the_term_depth_limit_on_a_small_stack() {
        use super::parser::parse_surface_tests::on_small_stack;
        let wrap = 60;
        let n = 200;
        let pattern: String = (0..n)
            .map(|i| {
                format!(
                    "{}?y{i}{} rdf:first ?y{} . ",
                    "(".repeat(wrap),
                    ")".repeat(wrap),
                    i + 1
                )
            })
            .collect();
        let src = format!(
            "@prefix : <http://ex/> .\n\
             @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n\
             :s :p :z .\n\
             {{ {{ :a :b :c }} log:includes {{ {pattern}}} }} => {{ :s :q ?y{n} }} .\n"
        );
        let (facts, cut) = on_small_stack(move || {
            let (facts, cut) = closure_and_cut(&src, None);
            (facts.map(|f| f.len()), cut)
        });
        assert_eq!(facts, Ok(1), "only the asserted fact is kept");
        assert!(cut.is_some());
    }

    /// #6732: a backward proof step checks its instantiated goal and the substitution it
    /// carries into the rule's premise against the term-depth limit before it evaluates
    /// that premise. A step past the limit is dropped as a cut, on a 2 MiB stack, and
    /// the run ends.
    #[test]
    fn backward_goals_stay_within_the_term_depth_limit_on_a_small_stack() {
        use super::parser::parse_surface_tests::on_small_stack;
        let wrap = 60;
        let src = format!(
            "@prefix : <http://ex/> .\n\
             :a :s :b .\n\
             {{ ?x :r ?y }} <= {{ {}?x{} :r ?y }} .\n\
             {{ :a :s ?b . :a :r ?y }} => {{ :a :q ?y }} .\n",
            "(".repeat(wrap),
            ")".repeat(wrap)
        );
        let (facts, cut) = on_small_stack(move || {
            let (facts, cut) = closure_and_cut(&src, None);
            (facts.map(|f| f.len()), cut)
        });
        assert_eq!(facts, Ok(1), "only the asserted fact is kept");
        assert!(cut.is_some());
    }

    /// #6732: a nested closure (`log:supports`, `log:conclusion`) checks the statements
    /// it is handed against the term-depth limit before it admits them as facts and
    /// rules. A formula past the limit is not closed: the refusal is a cut, on a 2 MiB
    /// stack, and only the formula's own statements are read.
    #[test]
    fn nested_closure_inputs_stay_within_the_term_depth_limit_on_a_small_stack() {
        use super::parser::parse_surface_tests::on_small_stack;
        let limit = super::bounded::term_depth();
        let (outer, inner) = (limit / 2 - 4, limit / 2 - 4);
        let wrapped = |w: usize, x: &str| format!("{}{x}{}", "(".repeat(w), ")".repeat(w));
        let src = format!(
            "@prefix : <http://ex/> .\n\
             @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             :s :p {} .\n\
             {{ :s :p ?x }} => {{ :s :p2 {} }} .\n\
             {{ :s :p2 ?y . {{ :t :r {} . {{ :t :r ?z }} => {{ :u :v :w }} }} log:supports {{ :u :v :w }} }} \
             => {{ :s :ok :yes }} .\n",
            wrapped(outer, ":z"),
            wrapped(inner, "?x"),
            wrapped(limit - outer - inner + 1, "?y"),
        );
        let (facts, cut) = on_small_stack(move || closure_and_cut(&src, None));
        let facts = facts.expect("closes");
        let ok = [
            Term::Iri("http://ex/s".into()),
            Term::Iri("http://ex/ok".into()),
            Term::Iri("http://ex/yes".into()),
        ];
        assert!(!facts.contains(&ok), "the formula is not closed");
        assert!(cut.is_some());
    }

    /// #6732: a top-level run keeps terms as deep as the deepest term it was handed, an
    /// empty innermost list or formula counted as a level of its own, so a rule copying
    /// such a term is not cut.
    #[test]
    fn a_rule_copies_given_terms_ending_in_an_empty_container_uncut() {
        let limit = super::bounded::term_depth();
        let iri = |s: &str| Term::Iri(format!("http://ex/{s}"));
        let outcomes: Vec<(&str, Option<&str>, bool)> = [
            ("empty list", Term::List(Vec::new())),
            ("empty formula", Term::Formula(Vec::new())),
        ]
        .into_iter()
        .map(|(kind, innermost)| {
            let mut deep = innermost;
            for _ in 0..limit {
                deep = Term::List(vec![deep]);
            }
            assert_eq!(depth_of(&deep), limit + 1);
            let run = Truncation::top_level();
            let rules = "{ <http://ex/s> <http://ex/p> ?x } => { <http://ex/s> <http://ex/q> ?x } .";
            let parsed = super::bounded::parse_n3(rules, "", &run).expect("parses");
            let docs = super::RuntimeDocs::for_sources(&[&parsed]);
            let closure = super::reason_n3_terms_with_facts(
                rules,
                [[iri("s"), iri("p"), deep.clone()]],
                NegationCycles::Reject,
                &run,
                &docs,
            )
            .expect("closes");
            (
                kind,
                run.get(),
                closure.facts.contains(&[iri("s"), iri("q"), deep]),
            )
        })
        .collect();
        // Each copy is kept, and nothing is cut.
        assert_eq!(
            outcomes,
            [("empty list", None, true), ("empty formula", None, true)]
        );
    }

    /// Every registry entry's declared store read, walked over the whole registry
    /// (`ALL` is generated with each enum, so a new variant joins this walk) and checked
    /// against the expected table below. A new builtin fails here until the table says
    /// what it reads.
    #[test]
    fn every_registry_entry_declares_its_store_read() {
        let expected: &[(&str, &str)] = &[
            ("Builtin", "Nothing"),
            ("Bidi::LogUri", "Nothing"),
            ("ListGen::Member", "ListCells(Subject)"),
            ("ListGen::Iterate", "ListCells(Subject)"),
            ("ListGen::In", "ListCells(Object)"),
            ("ScopeOp", "Scoped"),
            ("CollectOp", "Scoped"),
            ("Func::Dtlit", "Nothing"),
            ("Func::Append", "MemberListCells"),
            ("Func::LogConclusion", "Nested"),
            ("Func", "ListCells(Subject)"),
            ("VirtualRel", "VirtualList"),
        ];
        let want = |enum_name: &str, variant: &str| -> String {
            let full = format!("{enum_name}::{variant}");
            expected
                .iter()
                .find(|(k, _)| *k == full)
                .or_else(|| expected.iter().find(|(k, _)| *k == enum_name))
                .map(|(_, v)| v.to_string())
                .unwrap_or_else(|| panic!("no expected store read for {full}"))
        };
        let mut seen = 0;
        let mut check = |enum_name: &str, variant: String, got: StoreRead| {
            assert_eq!(
                format!("{got:?}"),
                want(enum_name, &variant),
                "{enum_name}::{variant}"
            );
            seen += 1;
        };
        for &v in Builtin::ALL {
            check("Builtin", format!("{v:?}"), v.store_read());
        }
        for &v in Bidi::ALL {
            check("Bidi", format!("{v:?}"), v.store_read());
        }
        for &v in ListGen::ALL {
            check("ListGen", format!("{v:?}"), v.store_read());
        }
        for &v in ScopeOp::ALL {
            check("ScopeOp", format!("{v:?}"), v.store_read());
        }
        for &v in CollectOp::ALL {
            check("CollectOp", format!("{v:?}"), v.store_read());
        }
        for &v in Func::ALL {
            check("Func", format!("{v:?}"), v.store_read());
        }
        for v in [
            VirtualRel::First,
            VirtualRel::Rest,
            VirtualRel::AnyPredicate,
        ] {
            check("VirtualRel", format!("{v:?}"), v.store_read());
        }
        assert!(seen > 90, "the registry walk covered {seen} entries");
        // Predicates resolve to the virtual relations and to stored joins.
        let iri = |i: &str| Term::Iri(i.into());
        assert!(matches!(
            relation(&iri(parser::RDF_FIRST)),
            Relation::Virtual(VirtualRel::First)
        ));
        assert!(matches!(
            relation(&iri(parser::RDF_REST)),
            Relation::Virtual(VirtualRel::Rest)
        ));
        assert!(matches!(
            relation(&Term::Var("p".into())),
            Relation::Virtual(VirtualRel::AnyPredicate)
        ));
        assert_eq!(relation(&iri("http://ex/p")).store_read(), StoreRead::Joins);
    }

    use super::*;

    /// #6735 review round 4: plain materialization never pays for proof identity — no
    /// statement is keyed on the `reason_n3` / stratified paths; the proof run keys every
    /// closure row.
    #[test]
    fn the_unkeyed_paths_never_compute_identity_keys() {
        let calls = || serialize::KEY_CALLS.with(|c| c.get());
        let src = "@prefix : <http://ex/>. :a :p ( 1 2 ). :b :p :c. { ?x :p ?y } => { ?y :q ?x }.";
        let before = calls();
        let n = reason_n3(&mut Dict::new(), src).expect("reasons").len();
        reason_n3_stratified(&mut Dict::new(), &[src, "@prefix : <http://ex/>. { ?x :q ?y } => { ?x :r ?y }."]).expect("reasons");
        assert_eq!(calls(), before, "the unkeyed paths keyed a statement");
        let (_, steps) = reason_n3_proof(&mut Dict::new(), src).expect("reasons");
        assert_eq!(calls() - before, steps.iter().map(|s| 1 + s.premises.len()).sum::<usize>(), "only the steps are keyed");
        let before = calls();
        let run = reason_n3_proof_run(&mut Dict::new(), src).expect("reasons");
        assert_eq!(run.closure_keys.len(), run.closure.len());
        assert!(calls() - before >= n, "the proof run keys every closure row");
    }

    fn closure(src: &str) -> (Dict, FxHashSet<[Id; 3]>) {
        let mut dict = Dict::new();
        let triples = reason_n3(&mut dict, src).unwrap();
        let set = triples.into_iter().collect();
        (dict, set)
    }
    fn id(dict: &Dict, iri: &str) -> Id {
        use oxrdf::{NamedNode, Term as OT};
        dict.lookup(&OT::NamedNode(NamedNode::new_unchecked(iri.to_string())))
    }
    fn has(dict: &Dict, set: &FxHashSet<[Id; 3]>, s: &str, p: &str, o: &str) -> bool {
        let (a, b, c) = (id(dict, s), id(dict, p), id(dict, o));
        a != 0 && b != 0 && c != 0 && set.contains(&[a, b, c])
    }

    /// The cut a run of `src` records, if any.
    fn run_cut(src: &str, resolver: Option<&Resolver>) -> Option<&'static str> {
        let truncated = Truncation::top_level();
        let parsed = parser::parse(src).expect("parse");
        let _ = run_closure(
            parsed,
            resolver,
            None,
            &truncated,
            StepMode::None,
            NegationCycles::Reject,
            None,
        );
        truncated.get()
    }

    /// Every limit kind records a cut on the run, at the top level and from inside a
    /// nested `log:conclusion` / `log:supports` closure (whose run shares the parent's
    /// record), and a document without a limit records none.
    #[test]
    fn every_limit_records_a_cut_on_the_run_including_nested_closures() {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(every_limit_records_a_cut)
            .expect("spawn")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    fn every_limit_records_a_cut() {
        let pre = "@prefix : <http://ex/> .\n\
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .\n\
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n\
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .\n\
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";
        let deep = format!(
            "<http://ex/a> <http://ex/b> {}1{} .",
            "(".repeat(5000),
            ")".repeat(5000)
        );
        let mut backward: String = (0..=64)
            .map(|i| format!("{{ :a :p{i} :b }} <= {{ :a :p{} :b }} .\n", i + 1))
            .collect();
        backward.push_str(":a :p65 :b .\n");
        let big: String = (0..8).map(|i| format!(":s{i} :p :o . ")).collect();
        // (limit, facts and backward rules, trigger premise)
        let cases: Vec<(&str, String, String)> = vec![
            ("backward depth", backward, ":a :p0 :b".into()),
            (
                "containment budget",
                String::new(),
                format!(
                    "{{ {big} }} log:includes {{ ?a :p ?c . ?d :p ?f . ?g :p ?i . ?j :p ?l . \
                     ?m :p ?n . ?o :p ?q . ?r :p :never }}"
                ),
            ),
            (
                "list walk cap",
                ":l rdf:first :x ; rdf:rest :l .\n".into(),
                ":l list:member :never".into(),
            ),
            ("regex", String::new(), "\"a\" string:matches \"(\"".into()),
            (
                "parser nesting in log:parsedAsN3",
                String::new(),
                format!("\"{deep}\" log:parsedAsN3 ?f"),
            ),
            (
                "parser nesting in log:semantics",
                String::new(),
                "<http://ex/deep> log:semantics ?f".into(),
            ),
            (
                "unresolvable log:semantics",
                String::new(),
                "<http://ex/missing> log:semantics ?f".into(),
            ),
            (
                "epoch year",
                String::new(),
                "\"1000000000-01-01T00:00:00Z\" time:inSeconds ?s".into(),
            ),
            (
                "exact arithmetic past i128",
                String::new(),
                format!("({} 1) math:sum ?x", i128::MAX),
            ),
            (
                "a fraction where only a whole number is exact",
                String::new(),
                "?d time:inSeconds 1.5".into(),
            ),
            (
                "epoch year past i64",
                String::new(),
                "\"9223372036854775808-01-01T00:00:00Z\" time:inSeconds ?s".into(),
            ),
        ];
        let resolve = move |iri: &str| (iri == "http://ex/deep").then(|| deep.clone());
        let resolver: &Resolver = &resolve;
        for (limit, facts, trigger) in &cases {
            let rule = format!("{{ {trigger} }} => {{ :a :proved :b }} .");
            let top = format!("{pre}{facts}{rule}");
            let conclusion =
                format!("{pre}{{ {{ {facts}{rule} }} log:conclusion ?c }} => {{ :x :y :z }} .");
            let supports = format!(
                "{pre}{{ {{ {facts}{rule} }} log:supports {{ :a :proved :b }} }} => {{ :x :y :z }} ."
            );
            for (shape, src) in [
                ("top level", top),
                ("log:conclusion", conclusion),
                ("log:supports", supports),
            ] {
                assert!(
                    run_cut(&src, Some(resolver)).is_some(),
                    "{limit} / {shape}: no cut recorded"
                );
            }
        }
        let clean = format!("{pre}:a :p1 :b .\n{{ :a :p1 :b }} => {{ :a :proved :b }} .");
        assert_eq!(run_cut(&clean, None), None);
    }

    #[test]
    fn proof_records_derivations() {
        // Chained rules: each derived triple should have a proof step naming its premise.
        let src = r#"
            @prefix : <http://ex/> .
            :Socrates a :Man .
            { ?x a :Man } => { ?x a :Mortal } .
            { ?x a :Mortal } => { ?x a :Being } .
        "#;
        let mut dict = Dict::new();
        let (_triples, proof) = reason_n3_proof(&mut dict, src).unwrap();
        // Two derivations: Socrates a Mortal (from Socrates a Man), Socrates a Being (from Mortal).
        assert_eq!(proof.len(), 2, "two derivation steps");
        let mortal = dict.intern_iri("http://ex/Mortal");
        let man = dict.intern_iri("http://ex/Man");
        let socrates = dict.intern_iri("http://ex/Socrates");
        let ty = dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let step = proof.iter().find(|s| s.conclusion == [socrates, ty, mortal]).expect("Mortal step");
        assert_eq!(step.premises, vec![[socrates, ty, man]], "Mortal derived from (Socrates a Man)");
    }

    #[test]
    fn simple_rule_socrates() {
        // The canonical N3 rule: every Man is Mortal.
        let src = r#"
            @prefix : <http://ex/> .
            :Socrates a :Man .
            { ?x a :Man } => { ?x a :Mortal } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/Socrates", "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", "http://ex/Mortal"));
    }

    #[test]
    fn backward_rule_arrow() {
        // `{ conclusion } <= { premise }` is GOAL-DIRECTED (EYE semantics): it never fires
        // forward on its own; a forward rule whose premise needs the conclusion (here an
        // EYE-style query rule `{goal} => {goal}`) triggers the backward proof.
        let src = r#"
            @prefix : <http://ex/> .
            :Socrates a :Man .
            { ?x a :Mortal } <= { ?x a :Man } .
            { ?x a :Mortal } => { ?x a :Mortal } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/Socrates", ty, "http://ex/Mortal"), "query rule drives backward proof");

        // Without a forward rule posing the goal, the backward rule must NOT materialize
        // anything — EYE outputs no `:Mortal` triple for this document either.
        let src_no_query = r#"
            @prefix : <http://ex/> .
            :Socrates a :Man .
            { ?x a :Mortal } <= { ?x a :Man } .
        "#;
        let (d2, s2) = closure(src_no_query);
        assert!(!has(&d2, &s2, "http://ex/Socrates", ty, "http://ex/Mortal"), "no goal, no backward firing");
    }

    #[test]
    fn backward_rule_builtin_premise() {
        // The eyereasoner/eye reasoning/backward case, inlined: the backward premise is a
        // PURE BUILTIN over the goal's variables — only evaluable once the query rule's
        // goal binds ?X/?Y (a forward reversal of `<=` derives nothing here).
        let src = r#"
            @prefix math: <http://www.w3.org/2000/10/swap/math#>.
            @prefix : <http://example.org/#>.
            { ?X :moreInterestingThan ?Y. } <= { ?X math:greaterThan ?Y. }.
            { 5 :moreInterestingThan 3. } => { 5 :moreInterestingThan 3. }.
        "#;
        let (mut d, s) = closure(src);
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let (five, three) = (d.intern_lit("5", int, None), d.intern_lit("3", int, None));
        assert!(
            s.contains(&[five, id(&d, "http://example.org/#moreInterestingThan"), three]),
            "5 :moreInterestingThan 3 proven goal-directed"
        );
    }

    #[test]
    fn backward_rule_chained_and_base_case() {
        // Backward rules chaining through other backward rules, plus a `<= true` base case.
        let src = r#"
            @prefix : <http://ex/> .
            :rex a :Dog .
            { ?x a :Animal } <= { ?x a :Dog } .
            { ?x :needs :food } <= { ?x a :Animal } .
            { :water a :Necessity } <= true .
            { ?x :needs :food } => { ?x :gets :food } .
            { :water a :Necessity } => { :water :is :necessary } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/rex", "http://ex/gets", "http://ex/food"), "two-step backward chain");
        assert!(has(&d, &s, "http://ex/water", "http://ex/is", "http://ex/necessary"), "<= true base case");
        // The intermediate backward conclusions are NOT materialized.
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(!has(&d, &s, "http://ex/rex", ty, "http://ex/Animal"), "backward conclusions stay virtual");
    }

    #[test]
    fn backward_rule_recursion_bounded() {
        // A self-referential backward rule must not hang: the depth bound cuts the cycle
        // and the engine still terminates (deriving nothing for the unprovable goal).
        let src = r#"
            @prefix : <http://ex/> .
            :seed :p :o .
            { ?x :loops ?y } <= { ?y :loops ?x } .
            { ?a :loops ?b } => { ?a :looped ?b } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/seed", "http://ex/p", "http://ex/o"), "facts survive");
        assert!(
            !s.iter().any(|[_, p, _]| *p == id(&d, "http://ex/looped")),
            "cyclic backward goal proves nothing"
        );
    }

    #[test]
    fn transitive_via_rule() {
        // Define transitivity with an N3 rule and close a chain.
        let src = r#"
            @prefix : <http://ex/> .
            :a :before :b . :b :before :c . :c :before :d .
            { ?x :before ?y . ?y :before ?z } => { ?x :before ?z } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/a", "http://ex/before", "http://ex/c"));
        assert!(has(&d, &s, "http://ex/a", "http://ex/before", "http://ex/d"));
        assert!(has(&d, &s, "http://ex/b", "http://ex/before", "http://ex/d"));
    }

    #[test]
    fn functional_math_sum_and_product() {
        // (?a ?b) math:sum ?s computes ?s; chained with product.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :rect :width 4 ; :height 5 .
            { ?r :width ?w . ?r :height ?h . (?w ?h) math:product ?area } => { ?r :area ?area } .
            { ?r :width ?w . ?r :height ?h . (?w ?h) math:sum ?half } => { ?r :perimeterHalf ?half } .
        "#;
        let (mut d, s) = closure(src);
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let area_id = d.intern_lit("20", int, None); // 4*5
        let half_id = d.intern_lit("9", int, None); // 4+5
        assert!(s.contains(&[id(&d, "http://ex/rect"), id(&d, "http://ex/area"), area_id]), "math:product 4*5=20");
        assert!(s.contains(&[id(&d, "http://ex/rect"), id(&d, "http://ex/perimeterHalf"), half_id]), "math:sum 4+5=9");
    }

    #[test]
    fn functional_string_length() {
        // ?w string:length ?n  — Unicode scalar count (T12).
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :a :word "hello" .
            { ?x :word ?w . ?w string:length ?n } => { ?x :wordLen ?n } .
        "#;
        let (mut d, s) = closure(src);
        let five = d.intern_lit("5", "http://www.w3.org/2001/XMLSchema#integer", None);
        assert!(s.contains(&[id(&d, "http://ex/a"), id(&d, "http://ex/wordLen"), five]), "string:length(hello)=5");
    }

    #[test]
    fn string_matches_regex() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :a :code "AB-123" . :b :code "xyz" .
            { ?x :code ?c . ?c string:matches "^[A-Z]+-[0-9]+$" } => { ?x a :Valid } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/a", "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", "http://ex/Valid"));
        assert!(!has(&d, &s, "http://ex/b", "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", "http://ex/Valid"));
    }

    #[test]
    fn string_replace_regex() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :a :raw "a1b2c3" .
            { ?x :raw ?r . ( ?r "[0-9]" "_" ) string:replace ?out } => { ?x :clean ?out } .
        "#;
        let (mut d, s) = closure(src);
        let expected = d.intern_lit("a_b_c_", "http://www.w3.org/2001/XMLSchema#string", None);
        assert!(s.contains(&[id(&d, "http://ex/a"), id(&d, "http://ex/clean"), expected]), "replace [0-9]->_ = a_b_c_");
    }

    #[test]
    fn functional_math_remainder_intquotient() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :x :v 17 .
            { ?x :v ?v . (?v 5) math:remainder ?r . (?v 5) math:integerQuotient ?q } => { ?x :rem ?r . ?x :quot ?q } .
        "#;
        let (mut d, s) = closure(src);
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        assert!(s.contains(&[id(&d, "http://ex/x"), id(&d, "http://ex/rem"), d.intern_lit("2", int, None)]), "17 mod 5 = 2");
        assert!(s.contains(&[id(&d, "http://ex/x"), id(&d, "http://ex/quot"), d.intern_lit("3", int, None)]), "17 div 5 = 3");
    }

    #[test]
    fn functional_list_first_last() {
        // ( … ) list:first / list:last over a rule-local collection (T12).
        let src = r#"
            @prefix : <http://ex/> .
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .
            { ( :a :b :c ) list:first ?f . ( :a :b :c ) list:last ?z } => { :s :first ?f . :s :last ?z } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/s", "http://ex/first", "http://ex/a"));
        assert!(has(&d, &s, "http://ex/s", "http://ex/last", "http://ex/c"));
    }

    #[test]
    fn path_syntax_forward() {
        // `?x!:mother :knows ?f` — the path ?x!:mother is the mother node; desugars to
        // (?x :mother _m)(_m :knows ?f).
        let src = r#"
            @prefix : <http://ex/> .
            :alice :mother :mary .
            :mary :knows :bob .
            { ?x!:mother :knows ?f } => { ?x :motherKnows ?f } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/alice", "http://ex/motherKnows", "http://ex/bob"), "forward path !");
    }

    #[test]
    fn path_syntax_backward() {
        // `?x^:mother` — the subject whose :mother is ?x (i.e. ?x's children).
        let src = r#"
            @prefix : <http://ex/> .
            :alice :mother :mary .
            { ?child :mother ?m . ?m^:mother :name ?cn } => { ?m :hasChildNamed ?cn } .
            :alice :name "Alice" .
        "#;
        // ?m^:mother is a child of ?m; simpler: verify ^ desugars to a backward triple.
        let (d, s) = closure(src);
        // ?m=mary: mary^:mother = alice (alice :mother mary); alice :name "Alice"
        // ⊢ mary :hasChildNamed "Alice".
        assert!(
            s.iter().any(|[a, p, _]| *a == id(&d, "http://ex/mary") && *p == id(&d, "http://ex/hasChildNamed")),
            "backward path ^ derived mary :hasChildNamed"
        );
    }

    #[test]
    fn scoped_negation_not_includes() {
        // log:notIncludes with an UNBOUND scope — negation as failure against
        // the store (the engine's documented idiom): a Person with no
        // recorded email is :NoEmail.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :alice a :Person .
            :bob a :Person .
            :bob :hasEmail "bob@x" .
            { ?x a :Person . ?store log:notIncludes { ?x :hasEmail ?e } } => { ?x a :NoEmail } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/alice", ty, "http://ex/NoEmail"), "alice has no email → NoEmail");
        assert!(!has(&d, &s, "http://ex/bob", ty, "http://ex/NoEmail"), "bob has email → excluded");
    }

    #[test]
    fn empty_formula_includes_nothing() {
        // `{}` is the EMPTY formula (cwm builtins.n3): it includes nothing —
        // not even true builtin atoms — and notIncludes everything.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :seed :p :o .
            { {} log:includes { :a log:equalTo :a } } => { :s a :Leak } .
            { {} log:notIncludes { :a log:equalTo :a } } => { :s a :Clean } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(!has(&d, &s, "http://ex/s", ty, "http://ex/Leak"), "empty formula includes nothing");
        assert!(has(&d, &s, "http://ex/s", ty, "http://ex/Clean"), "empty formula notIncludes everything");
    }

    #[test]
    fn includes_quantifier_matrix() {
        // The cwm quantifiers_limited matrix: pattern existentials are
        // wildcards; scope quantified terms are opaque constants.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :seed :p :o .
            { { :foo :bar :baz } log:includes { @forSome :foo . :foo :bar :baz } } => { :a2 a :S } .
            { { @forAll :foo . :foo :bar :baz } log:includes { @forSome :foo . :foo :bar :baz } } => { :c2 a :S } .
            { { @forSome :foo . :foo :bar :baz } log:includes { :foo :bar :baz } } => { :b1 a :S } .
            { { @forAll :foo . :foo :bar :baz } log:includes { :foo :bar :baz } } => { :c1 a :S } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/a2", ty, "http://ex/S"), "existential pattern matches ground scope");
        assert!(has(&d, &s, "http://ex/c2", ty, "http://ex/S"), "existential pattern matches universal scope");
        assert!(!has(&d, &s, "http://ex/b1", ty, "http://ex/S"), "ground pattern vs existential scope: no");
        assert!(!has(&d, &s, "http://ex/c1", ty, "http://ex/S"), "ground pattern vs universal scope: no (cwm)");
    }

    #[test]
    fn log_supports_closes_scope() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :seed :p :o .
            { { :a :b :c . { :a :b :c } => { :d :e :f } } log:supports { :a :b :c . :d :e :f } }
              => { :q a :S } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/q", ty, "http://ex/S"), "supports = containment in the scope's closure");
    }

    #[test]
    fn string_contains_filter() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :a :label "hello world" .
            :b :label "goodbye" .
            { ?x :label ?l . ?l string:contains "world" } => { ?x a :Matched } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/a", ty, "http://ex/Matched"), "string:contains match");
        assert!(!has(&d, &s, "http://ex/b", ty, "http://ex/Matched"), "non-match excluded");
    }

    #[test]
    fn list_member_generator() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .
            :s :p :o .
            { ( :a :b :c ) list:member ?x } => { ?x a :Listed } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        for m in ["a", "b", "c"] {
            assert!(has(&d, &s, &format!("http://ex/{m}"), ty, "http://ex/Listed"), "list:member {m}");
        }
    }

    #[test]
    fn time_components_and_unary_math() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :e :when "2024-03-15T10:30:45"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
            :e :temp -5 .
            { ?x :when ?d . ?d time:year ?y . ?d time:month ?mo . ?d time:day ?dd } => { ?x :y ?y ; :mo ?mo ; :dd ?dd } .
            { ?x :temp ?t . ?t math:absoluteValue ?a } => { ?x :absTemp ?a } .
        "#;
        let (mut d, s) = closure(src);
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let lit = |d: &mut Dict, n: &str| d.intern_lit(n, int, None);
        let (y, mo, dd, a) = (lit(&mut d, "2024"), lit(&mut d, "3"), lit(&mut d, "15"), lit(&mut d, "5"));
        let e = id(&d, "http://ex/e");
        assert!(s.contains(&[e, id(&d, "http://ex/y"), y]), "time:year = 2024");
        assert!(s.contains(&[e, id(&d, "http://ex/mo"), mo]), "time:month = 3");
        assert!(s.contains(&[e, id(&d, "http://ex/dd"), dd]), "time:day = 15");
        assert!(s.contains(&[e, id(&d, "http://ex/absTemp"), a]), "math:absoluteValue(-5) = 5");
    }

    #[test]
    fn math_max_and_list_length() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .
            :d :seed "x" .
            { ( 3 7 2 ) math:max ?m . ( 3 7 2 ) list:length ?n } => { :d :maxVal ?m ; :count ?n } .
        "#;
        let (mut d, s) = closure(src);
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let seven = d.intern_lit("7", int, None);
        let three = d.intern_lit("3", int, None);
        assert!(s.contains(&[id(&d, "http://ex/d"), id(&d, "http://ex/maxVal"), seven]), "math:max = 7");
        assert!(s.contains(&[id(&d, "http://ex/d"), id(&d, "http://ex/count"), three]), "list:length = 3");
    }

    #[test]
    fn list_member_with_bound_variables() {
        // list:member resolves members THROUGH the current binding: ( ?v :extra ) with ?v
        // joined from data generates one binding per (substituted) member.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .
            :s :p :o1 . :s :p :o2 .
            { :s :p ?v . ( ?v :extra ) list:member ?m } => { ?m a :Seen } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        for m in ["o1", "o2", "extra"] {
            assert!(has(&d, &s, &format!("http://ex/{m}"), ty, "http://ex/Seen"), "list:member {m}");
        }
    }

    #[test]
    fn list_in_generator() {
        // ?x list:in ( … ) — the inverse direction of list:member.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .
            :seed :p :o .
            { ?x list:in ( :a :b ) } => { ?x a :InList } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/a", ty, "http://ex/InList"), "list:in a");
        assert!(has(&d, &s, "http://ex/b", ty, "http://ex/InList"), "list:in b");
        assert!(!has(&d, &s, "http://ex/o", ty, "http://ex/InList"), "non-member excluded");
    }

    #[test]
    fn string_lower_upper_case() {
        // string:lowerCase / string:upperCase — functional, Unicode-aware case mapping.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :a :word "HeLLo Wörld" .
            { ?x :word ?w . ?w string:lowerCase ?l . ?w string:upperCase ?u } => { ?x :lower ?l ; :upper ?u } .
        "#;
        let (mut d, s) = closure(src);
        let xs = "http://www.w3.org/2001/XMLSchema#string";
        let lo = d.intern_lit("hello wörld", xs, None);
        let up = d.intern_lit("HELLO WÖRLD", xs, None);
        assert!(s.contains(&[id(&d, "http://ex/a"), id(&d, "http://ex/lower"), lo]), "string:lowerCase");
        assert!(s.contains(&[id(&d, "http://ex/a"), id(&d, "http://ex/upper"), up]), "string:upperCase");
    }

    /// #3712 tripwire: every printable ASCII byte + a control + 2-/4-byte UTF-8 — exactly the
    /// RFC 3986 unreserved set passes through; the rest is uppercase `%XX` per UTF-8 byte.
    #[test]
    fn encode_for_uri_pins_the_unreserved_set() {
        let all: String = (0x20u8..0x7f)
            .map(char::from)
            .chain("\t\né😀".chars())
            .collect();
        assert_eq!(
            encode_for_uri(&all),
            "%20%21%22%23%24%25%26%27%28%29%2A%2B%2C-.%2F0123456789%3A%3B%3C%3D%3E%3F%40ABCDEFGHIJKLMNOPQRSTUVWXYZ%5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D~%09%0A%C3%A9%F0%9F%98%80"
        );
    }

    #[test]
    fn string_encode_for_uri() {
        // string:encodeForUri — RFC 3986 percent-encoding (fn:encode-for-uri set):
        // unreserved untouched, delimiters/space encoded, non-ASCII as UTF-8 bytes
        // with uppercase hex. Object-ground use acts as an equality filter.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :a :raw "AZaz09-._~" .
            :b :raw "https://alice.ex/card#me&client=x" .
            :c :raw "café déjà" .
            { ?x :raw ?r . ?r string:encodeForUri ?e } => { ?x :enc ?e } .
            { ?x :raw ?r . ?r string:encodeForUri "AZaz09-._~" } => { ?x a :Unchanged } .
        "#;
        let (mut d, s) = closure(src);
        let xs = "http://www.w3.org/2001/XMLSchema#string";
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        let ea = d.intern_lit("AZaz09-._~", xs, None);
        let eb = d.intern_lit("https%3A%2F%2Falice.ex%2Fcard%23me%26client%3Dx", xs, None);
        let ec = d.intern_lit("caf%C3%A9%20d%C3%A9j%C3%A0", xs, None);
        assert!(s.contains(&[id(&d, "http://ex/a"), id(&d, "http://ex/enc"), ea]), "unreserved untouched");
        assert!(s.contains(&[id(&d, "http://ex/b"), id(&d, "http://ex/enc"), eb]), "delimiters encoded");
        assert!(s.contains(&[id(&d, "http://ex/c"), id(&d, "http://ex/enc"), ec]), "UTF-8 bytes, uppercase hex");
        assert!(has(&d, &s, "http://ex/a", ty, "http://ex/Unchanged"), "ground object: filter passes");
        assert!(!has(&d, &s, "http://ex/b", ty, "http://ex/Unchanged"), "ground object: filter rejects");
        // the helper directly (shared with sparq-solid's session-side pair minting)
        assert_eq!(super::encode_for_uri(" %"), "%20%25", "space and percent themselves");
        assert_eq!(super::encode_for_uri("नमस्ते"), "%E0%A4%A8%E0%A4%AE%E0%A4%B8%E0%A5%8D%E0%A4%A4%E0%A5%87");
    }

    #[test]
    fn string_not_matches_regex() {
        // string:notMatches — negated regex filter.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :a :code "abc" . :b :code "123" .
            { ?x :code ?c . ?c string:notMatches "^[0-9]+$" } => { ?x a :NonNumeric } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/a", ty, "http://ex/NonNumeric"), "abc does not match digits");
        assert!(!has(&d, &s, "http://ex/b", ty, "http://ex/NonNumeric"), "123 matches → excluded");
    }

    #[test]
    fn log_includes_ground_formula() {
        // Ground-formula containment: { f } log:includes { pattern } matches the pattern
        // against the SUBJECT formula's triples (not the store) and binds its variables.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :x :q :y .
            { { :a :p :b . :c :p :d } log:includes { ?s :p :b } } => { ?s a :FoundInFormula } .
            { { :a :p :b } log:includes { :x :q :y } } => { :s a :StoreLeak } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/a", ty, "http://ex/FoundInFormula"), "?s bound inside the formula");
        assert!(!has(&d, &s, "http://ex/c", ty, "http://ex/FoundInFormula"), ":c :p :d does not match ?s :p :b");
        // :x :q :y IS in the store but NOT in the subject formula — must not leak through.
        assert!(!has(&d, &s, "http://ex/s", ty, "http://ex/StoreLeak"), "formula scope must not see the store");
    }

    #[test]
    fn log_not_includes_ground_formula() {
        // notIncludes over a ground subject formula: containment failure, scoped to it.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :seed :p :o .
            { { :a :p :b } log:notIncludes { :a :p :c } } => { :s :notInc :yes } .
            { { :a :p :b } log:notIncludes { :a :p :b } } => { :s :bad :yes } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/s", "http://ex/notInc", "http://ex/yes"), "absent triple → notIncludes holds");
        assert!(!has(&d, &s, "http://ex/s", "http://ex/bad", "http://ex/yes"), "present triple → notIncludes fails");
    }

    #[test]
    fn trig_exact_values() {
        // The EYE trig family at exact points: sin 0 = 0, cos 0 = 1, sin(π/2) = 1, tan 0 = 0,
        // acos 1 = 0, sinh 0 = 0, cosh 0 = 1, tanh 0 = 0 (all exact in IEEE f64).
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :t :seed 0 .
            { ?x :seed ?z .
              ?z math:sin ?s . ?z math:cos ?c . ?z math:tan ?t .
              ?z math:sinh ?sh . ?z math:cosh ?ch . ?z math:tanh ?th .
              1 math:acos ?ac . 1.5707963267948966 math:sin ?shalf }
            => { ?x :sin ?s ; :cos ?c ; :tan ?t ; :sinh ?sh ; :cosh ?ch ; :tanh ?th ;
                    :acos1 ?ac ; :sinHalfPi ?shalf } .
        "#;
        let (mut d, s) = closure(src);
        // The real-valued (trig) family is double-typed, cwm-style e-notation.
        let dbl = "http://www.w3.org/2001/XMLSchema#double";
        let zero = d.intern_lit("0.0e0", dbl, None);
        let one = d.intern_lit("1.0e0", dbl, None);
        let t = id(&d, "http://ex/t");
        for (p, v) in [
            ("sin", zero), ("cos", one), ("tan", zero), ("sinh", zero), ("cosh", one),
            ("tanh", zero), ("acos1", zero), ("sinHalfPi", one),
        ] {
            assert!(s.contains(&[t, id(&d, &format!("http://ex/{p}")), v]), "math:{p} exact value");
        }
    }

    #[test]
    fn trig_domain_error_fails_premise() {
        // asin 2 is NaN — the premise must FAIL (no binding), not emit a NaN literal.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :seed :p :o .
            { 2 math:asin ?x } => { :s :bad ?x } .
        "#;
        let (d, s) = closure(src);
        assert!(!s.iter().any(|[a, p, _]| *a == id(&d, "http://ex/s") && *p == id(&d, "http://ex/bad")));
    }

    #[test]
    fn degrees_radians_roundtrip() {
        // π rad math:degrees 180; 180° math:radians π (both exact in f64, same ops as eye.pl).
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :a :rad 3.141592653589793 . :a :deg 180 .
            { ?x :rad ?r . ?r math:degrees ?d } => { ?x :inDegrees ?d } .
            { ?x :deg ?g . ?g math:radians ?r2 } => { ?x :inRadians ?r2 } .
        "#;
        let (mut d, s) = closure(src);
        let dbl = "http://www.w3.org/2001/XMLSchema#double";
        let deg = d.intern_lit("1.8e2", dbl, None);
        let rad = d.intern_lit("3.141592653589793e0", dbl, None);
        let a = id(&d, "http://ex/a");
        assert!(s.contains(&[a, id(&d, "http://ex/inDegrees"), deg]), "π rad = 180°");
        assert!(s.contains(&[a, id(&d, "http://ex/inRadians"), rad]), "180° = π rad");
    }

    #[test]
    fn logarithm_and_atan2() {
        // (x base) math:logarithm log_base(x); (x y) math:atan2 = atan(x/y) (EYE's impl).
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :seed :p :o .
            { (8 2) math:logarithm ?l . (1024 2) math:logarithm ?k . (0 1) math:atan2 ?a }
            => { :s :log8 ?l ; :log1024 ?k ; :atan ?a } .
            { (5 1) math:logarithm ?bad } => { :s :badLog ?bad } .
        "#;
        let (mut d, s) = closure(src);
        let dbl = "http://www.w3.org/2001/XMLSchema#double";
        let st = id(&d, "http://ex/s");
        let three = d.intern_lit("3.0e0", dbl, None);
        let ten = d.intern_lit("1.0e1", dbl, None);
        let zero = d.intern_lit("0.0e0", dbl, None);
        assert!(s.contains(&[st, id(&d, "http://ex/log8"), three]), "log_2 8 = 3");
        assert!(s.contains(&[st, id(&d, "http://ex/log1024"), ten]), "log_2 1024 = 10");
        assert!(s.contains(&[st, id(&d, "http://ex/atan"), zero]), "atan2(0,1) = 0");
        assert!(!s.iter().any(|[a, p, _]| *a == st && *p == id(&d, "http://ex/badLog")), "base 1 fails");
    }

    #[test]
    fn string_format_subset() {
        // %s / %d / %% work; an unsupported directive fails the premise (no mangling).
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :seed :p :o .
            { ("%s scored %d%%" "alice" 95) string:format ?out } => { :s :msg ?out } .
            { ("%x" 255) string:format ?bad } => { :s :hex ?bad } .
        "#;
        let (mut d, s) = closure(src);
        let msg = d.intern_lit("alice scored 95%", "http://www.w3.org/2001/XMLSchema#string", None);
        let st = id(&d, "http://ex/s");
        assert!(s.contains(&[st, id(&d, "http://ex/msg"), msg]), "%s/%d/%% formatting");
        assert!(!s.iter().any(|[a, p, _]| *a == st && *p == id(&d, "http://ex/hex")), "%x unsupported → fails");
    }

    #[test]
    fn string_scrape_capture_group() {
        // EYE biP.n3 strs1: the FIRST capture group of the first match, as xsd:string.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :seed :p :o .
            { ("http://example.org/1995/manifesto" "http://([^/]+)/([^/]+)") string:scrape ?h }
            => { :s :host ?h } .
        "#;
        let (mut d, s) = closure(src);
        let host = d.intern_lit("example.org", "http://www.w3.org/2001/XMLSchema#string", None);
        assert!(s.contains(&[id(&d, "http://ex/s"), id(&d, "http://ex/host"), host]), "capture group 1");
    }

    #[test]
    fn log_conjunction_merges_formulae() {
        // EYE biP.n3 logc2/logc3 semantics: merge a list of formulae (empties contribute
        // nothing); the result can be a log:includes scope in the same premise.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :seed :p :o .
            { ({} {:u :v :w} {:x :y :z. :j :k :l}) log:conjunction {:u :v :w. :x :y :z. :j :k :l} }
            => { :s :merged :exact } .
            { ( {:a :p :b} {:c :q :d} ) log:conjunction ?F . ?F log:includes { ?s :q :d } }
            => { ?s a :FoundViaConjunction } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/s", "http://ex/merged", "http://ex/exact"), "exact merge (logc2)");
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/c", ty, "http://ex/FoundViaConjunction"), "merged formula as scope");
    }

    #[test]
    fn log_uri_both_directions() {
        // forward: IRI → its text as xsd:string; reverse: string → the IRI it names.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :alice :knows :bob .
            { :alice log:uri ?u } => { :r :uriStr ?u } .
            { ?who log:uri "http://ex/bob" } => { ?who a :Named } .
        "#;
        let (mut d, s) = closure(src);
        let u = d.intern_lit("http://ex/alice", "http://www.w3.org/2001/XMLSchema#string", None);
        assert!(s.contains(&[id(&d, "http://ex/r"), id(&d, "http://ex/uriStr"), u]), "IRI → string");
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/bob", ty, "http://ex/Named"), "string → IRI");
    }

    #[test]
    fn log_dtlit_both_directions() {
        // forward: ( "lex" xsd:dt ) → "lex"^^xsd:dt; reverse: decompose a ground literal.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            :e :when "2024-03-15T10:30:45"^^xsd:dateTime .
            { ("2024-01-01" xsd:date) log:dtlit ?lit . ?lit time:year ?y } => { :d :y ?y } .
            { ?x :when ?w . (?lex ?dt) log:dtlit ?w } => { ?x :lexOf ?lex ; :dtOf ?dt } .
        "#;
        let (mut d, s) = closure(src);
        let y = d.intern_lit("2024", "http://www.w3.org/2001/XMLSchema#integer", None);
        assert!(s.contains(&[id(&d, "http://ex/d"), id(&d, "http://ex/y"), y]), "forward dtlit feeds time:year");
        let lex = d.intern_lit("2024-03-15T10:30:45", "http://www.w3.org/2001/XMLSchema#string", None);
        let e = id(&d, "http://ex/e");
        assert!(s.contains(&[e, id(&d, "http://ex/lexOf"), lex]), "reverse: lexical part");
        assert!(
            s.contains(&[e, id(&d, "http://ex/dtOf"), id(&d, "http://www.w3.org/2001/XMLSchema#dateTime")]),
            "reverse: datatype part"
        );
    }

    #[test]
    fn math_member_count() {
        // EYE biP.n3 mathm1/mathm2: list length, or DISTINCT triple count of a formula.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :seed :p :o .
            { (:u :v :u) math:memberCount ?n } => { :s :listCount ?n } .
            { {:s :p :o1. :s :p :o2. :s :p :o1} math:memberCount ?m } => { :s :graphCount ?m } .
        "#;
        let (mut d, s) = closure(src);
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let st = id(&d, "http://ex/s");
        assert!(s.contains(&[st, id(&d, "http://ex/listCount"), d.intern_lit("3", int, None)]), "list len 3");
        assert!(s.contains(&[st, id(&d, "http://ex/graphCount"), d.intern_lit("2", int, None)]), "2 distinct");
    }

    #[test]
    fn string_contains_ignoring_case() {
        // EYE biP.n3 strci1: "Tim" string:containsIgnoringCase "IM".
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :a :name "Tim" . :b :name "Bob" .
            { ?x :name ?n . ?n string:containsIgnoringCase "IM" } => { ?x a :Match } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/a", ty, "http://ex/Match"), "case-insensitive hit");
        assert!(!has(&d, &s, "http://ex/b", ty, "http://ex/Match"), "non-match excluded");
    }

    #[test]
    fn first_class_list_unification() {
        // `( ?x )` unifies structurally with a data list `( 17 )` (cwm unify2).
        let src = r#"
            @prefix : <http://ex/> .
            ( 17 ) a :TestCase .
            { ( ?x ) a :TestCase } => { ?x a :RESULT } .
        "#;
        let (mut d, s) = closure(src);
        let i17 = d.intern_lit("17", "http://www.w3.org/2001/XMLSchema#integer", None);
        let ty = id(&d, "http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        assert!(s.contains(&[i17, ty, id(&d, "http://ex/RESULT")]), "list unification binds ?x=17");
    }

    #[test]
    fn list_append_builtin() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .
            :seed :p :o .
            { ((1 2) (3)) list:append (1 2 3) } => { :s a :AppendOk } .
            { (() (1)) list:append (1) } => { :s a :EmptyOk } .
            { ((:a) (:b)) list:append ?out . ?out list:member ?m } => { ?m a :Member } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/s", ty, "http://ex/AppendOk"), "append filter");
        assert!(has(&d, &s, "http://ex/s", ty, "http://ex/EmptyOk"), "empty list append");
        assert!(has(&d, &s, "http://ex/a", ty, "http://ex/Member"), "constructed list is iterable");
        assert!(has(&d, &s, "http://ex/b", ty, "http://ex/Member"), "constructed list is iterable");
    }

    #[test]
    fn list_iterate_and_virtual_first_rest() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .
            ((:q)) a :Thing .
            { (:a :b) list:iterate (1 ?v) } => { ?v a :Second } .
            { ?X a :Thing . ?X rdf:rest ?Y } => { ?Y a :Thing } .
            { ?X a :Thing; rdf:first (?B) } => { ?B a :GreatThing } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(has(&d, &s, "http://ex/b", ty, "http://ex/Second"), "list:iterate index/value");
        assert!(has(&d, &s, "http://ex/q", ty, "http://ex/GreatThing"), "virtual rdf:first over list");
    }

    /// GH #6745: the `math:` comparisons and the bound-object value check of a functional
    /// `math:` builtin follow XPath numeric type promotion by DATATYPE — exact for
    /// integer/decimal pairs (past 2^53 and past i128), `xsd:float` when the other side
    /// is integer/decimal/float, `xsd:double` when either side is a double; `NaN` is
    /// unordered; an ill-typed operand never matches.
    #[test]
    fn math_comparisons_follow_xpath_numeric_promotion() {
        let cases: &[(&str, bool)] = &[
            // integer vs integer: exact at arbitrary precision.
            ("9007199254740993 math:equalTo 9007199254740992", false),
            ("9007199254740993 math:greaterThan 9007199254740992", true),
            ("9007199254740993 math:notEqualTo 9007199254740992", true),
            ("170141183460469231731687303715884105728 math:greaterThan 170141183460469231731687303715884105727", true),
            ("170141183460469231731687303715884105728 math:equalTo 170141183460469231731687303715884105727", false),
            // derived integer types are integers.
            ("\"5\"^^xsd:int math:greaterThan \"4\"^^xsd:byte", true),
            ("\"5\"^^xsd:long math:equalTo 5.0", true),
            // integer + decimal -> decimal: exact.
            ("1 math:equalTo 1.0", true),
            ("9007199254740993 math:greaterThan 9007199254740992.5", true),
            ("0.30000000000000000001 math:greaterThan 0.3", true),
            ("0.30000000000000000001 math:equalTo 0.3", false),
            ("0.3 math:lessThan 0.30000000000000000001", true),
            ("0.3 math:notLessThan 0.30000000000000000001", false),
            ("0.30 math:notGreaterThan 0.3", true),
            // decimal / integer + float -> float.
            ("0.1 math:equalTo \"0.1\"^^xsd:float", true),
            ("0.1 math:lessThan \"0.1\"^^xsd:float", false),
            ("16777217 math:equalTo \"16777216\"^^xsd:float", true),
            ("\"1.5\"^^xsd:float math:greaterThan 1.25", true),
            // float + double -> double (the float widens exactly).
            ("\"0.1\"^^xsd:float math:equalTo \"0.1\"^^xsd:double", false),
            ("\"0.1\"^^xsd:float math:greaterThan \"0.1\"^^xsd:double", true),
            ("\"0.5\"^^xsd:float math:equalTo 0.5e0", true),
            // integer / decimal + double -> double.
            ("9007199254740993 math:equalTo 9007199254740992e0", true),
            ("\"1\"^^xsd:double math:equalTo 1.0", true),
            ("0.30000000000000000001 math:equalTo 0.3e0", true),
            // NaN is unordered: only notEqualTo holds.
            ("\"NaN\"^^xsd:double math:equalTo \"NaN\"^^xsd:double", false),
            ("\"NaN\"^^xsd:double math:notEqualTo \"NaN\"^^xsd:double", true),
            ("\"NaN\"^^xsd:float math:notGreaterThan 1", false),
            ("\"NaN\"^^xsd:double math:notLessThan 1", false),
            ("\"INF\"^^xsd:double math:greaterThan 170141183460469231731687303715884105728", true),
            // An ill-typed operand never matches (not even notEqualTo).
            ("\"1.5\"^^xsd:integer math:notEqualTo 2", false),
            ("\"-1\"^^xsd:positiveInteger math:lessThan 0", false),
            ("\"1e0\"^^xsd:decimal math:equalTo 1", false),
            // A plain string is still coerced by its lexical shape (EYE).
            ("\"10\" math:greaterThan 9", true),
            ("\"9007199254740993\" math:greaterThan 9007199254740992", true),
            // The bound-object value check of a functional builtin promotes the same way.
            ("(16777216 1) math:sum \"16777216\"^^xsd:float", true),
            ("(16777216 1) math:sum 16777216", false),
            ("(9007199254740992 1) math:sum \"9007199254740992\"^^xsd:double", true),
            ("(1 2) math:sum \"3\"^^xsd:double", true),
            ("(0.1 0.2) math:sum 0.3", true),
            // XSD whiteSpace=collapse: a typed numeric literal's leading/trailing XML
            // whitespace is collapsed; an internal space stays ill-typed.
            ("(1 2) math:sum \" 3\"^^xsd:integer", true),
            ("(1 2) math:sum \"3\\n\"^^xsd:decimal", true),
            ("\" 3\"^^xsd:integer math:equalTo 3", true),
            ("\"\\t3.0 \"^^xsd:double math:equalTo 3", true),
            ("\" 3\"^^xsd:float math:lessThan 4", true),
            ("\"3 4\"^^xsd:integer math:notEqualTo 3", false),
            ("(1 2) math:sum \"3 4\"^^xsd:integer", false),
            // A string bound object is trimmed as before (numval coercion)...
            ("(1 2) math:sum \" 3\"", true),
            ("(1 2) math:sum \"3 \"", true),
            // ...but a string comparison operand is not: `\" 3\"` is not numeric there.
            ("\" 3\" math:equalTo 3", false),
            ("\" 3\" math:lessThan 4", false),
            ("\" 3\" math:notEqualTo 4", false),
        ];
        let mut src = String::from(
            "@prefix : <http://ex/> .\n@prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n",
        );
        for (i, (premise, _)) in cases.iter().enumerate() {
            src += &format!("{{ {premise} }} => {{ :t :fired :c{i} }} .\n");
        }
        let (d, s) = closure(&src);
        for (i, (premise, fires)) in cases.iter().enumerate() {
            let got = has(
                &d,
                &s,
                "http://ex/t",
                "http://ex/fired",
                &format!("http://ex/c{i}"),
            );
            assert_eq!(got, *fires, "{premise}");
        }
    }

    /// GH #6745 follow-up: a functional `math:` builtin whose f64 fallback overflows renders
    /// its non-finite result through `number_term` as `"inf"` / `"-inf"`^^xsd:decimal (the
    /// lexical forms are unchanged). The bound-object check and the `math:` comparisons read
    /// those exact rendered forms as ±INF / NaN, as the pre-promotion `numval` did, both
    /// against a ground object and on a variable bound to the result.
    #[test]
    fn math_nonfinite_fallback_results_compare_numerically() {
        // 0.5^-1024 = 2^1024 overflows; (-0.5)^-1025 = -(2^1025) overflows negative;
        // 0^-1 is +INF; 10^400 overflows the exact tower (a cut) and then f64; nine
        // factors of 10^38 overflow the exact product (a cut) and then f64.
        let big = "100000000000000000000000000000000000000";
        let product = format!("({})", [big; 9].join(" "));
        let cases: Vec<(String, bool)> = vec![
            // A ground object.
            ("(0.5 -1024) math:exponentiation \"INF\"^^xsd:double".into(), true),
            ("(0.5 -1024) math:exponentiation \"inf\"^^xsd:decimal".into(), true),
            ("(0.5 -1024) math:exponentiation \"-INF\"^^xsd:double".into(), false),
            ("(0.5 -1024) math:exponentiation 1".into(), false),
            ("(-0.5 -1025) math:exponentiation \"-INF\"^^xsd:double".into(), true),
            ("(-0.5 -1025) math:exponentiation \"INF\"^^xsd:double".into(), false),
            ("(10 400) math:exponentiation \"INF\"^^xsd:double".into(), true),
            ("(0 -1) math:exponentiation \"INF\"^^xsd:double".into(), true),
            (format!("{product} math:product \"INF\"^^xsd:double"), true),
            // A variable bound to the result, compared afterwards.
            ("(0.5 -1024) math:exponentiation ?x . ?x math:greaterThan 1".into(), true),
            ("(0.5 -1024) math:exponentiation ?x . ?x math:lessThan 1".into(), false),
            ("(0.5 -1024) math:exponentiation ?x . ?x math:equalTo \"INF\"^^xsd:double".into(), true),
            ("(0.5 -1024) math:exponentiation ?x . ?x math:notLessThan ?x".into(), true),
            ("(-0.5 -1025) math:exponentiation ?x . ?x math:lessThan -1".into(), true),
            ("(-0.5 -1025) math:exponentiation ?x . ?x math:equalTo \"-INF\"^^xsd:double".into(), true),
            ("(10 400) math:exponentiation ?x . ?x math:greaterThan 1".into(), true),
            ("(0 -1) math:exponentiation ?x . ?x math:greaterThan 1".into(), true),
            (format!("{product} math:product ?x . ?x math:greaterThan 1"), true),
            // INF - INF is NaN (a double, since an INF operand is a double): unordered.
            ("(0.5 -1024) math:exponentiation ?x . (?x ?x) math:difference ?n . ?n math:notEqualTo ?n".into(), true),
            ("(0.5 -1024) math:exponentiation ?x . (?x ?x) math:difference ?n . ?n math:equalTo ?n".into(), false),
            // The exact rendered forms read the same as written literals.
            ("\"inf\"^^xsd:decimal math:greaterThan 1".into(), true),
            ("\"-inf\"^^xsd:decimal math:lessThan -1".into(), true),
            ("\"NaN\"^^xsd:decimal math:notEqualTo 1".into(), true),
            ("\"NaN\"^^xsd:decimal math:equalTo \"NaN\"^^xsd:decimal".into(), false),
        ];
        let mut src = String::from(
            "@prefix : <http://ex/> .\n@prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n",
        );
        for (i, (premise, _)) in cases.iter().enumerate() {
            src += &format!("{{ {premise} }} => {{ :t :fired :c{i} }} .\n");
        }
        let (d, s) = closure(&src);
        let wrong: Vec<_> = (cases.iter().enumerate())
            .filter(|(i, (_, fires))| {
                has(&d, &s, "http://ex/t", "http://ex/fired", &format!("http://ex/c{i}")) != *fires
            })
            .map(|(_, (premise, fires))| format!("{premise} (expected {fires})"))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn math_builtin_filter() {
        // math:greaterThan as a premise filter: adults are people over 17.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :alice :age 30 . :bob :age 12 .
            { ?p :age ?a . ?a math:greaterThan 17 } => { ?p a :Adult } .
        "#;
        let (d, s) = closure(src);
        assert!(has(&d, &s, "http://ex/alice", "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", "http://ex/Adult"), "alice is adult");
        assert!(!has(&d, &s, "http://ex/bob", "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", "http://ex/Adult"), "bob is NOT adult");
    }

    // [OPUS-4.8] sq-fodw5 (sq-qcnn): correctness tests for the dark backward-chaining
    // (`<=`) projection + standardize-apart paths and the IEEE INF/NaN math fallback.
    // Every assertion is an EYE-semantics entailment derived by hand, not a line-exercise.

    #[test]
    fn backward_rule_binds_goal_to_list() {
        // A backward (`<=`) rule whose proof binds a GOAL variable to a LIST value, driving
        // the deep-resolve projection in `backward_prove` (walk the chain, then substitute
        // inside list structure) and `rename_vars`/`unify_walked` over `Term::List`.
        // :alice has a children list; the backward rule projects that list onto the query
        // goal's variable, and a forward query rule consumes it.
        let src = r#"
            @prefix : <http://ex/> .
            :alice :childrenList ( :kid1 :kid2 ) .
            { ?p :family ?kids } <= { ?p :childrenList ?kids } .
            { :alice :family ?ks } => { :alice :hasFamily ?ks . :marker a :Proven } .
        "#;
        let (d, s) = closure(src);
        // The backward proof must have bound ?ks to the ( :kid1 :kid2 ) list and projected it,
        // so the forward query rule fires and asserts the marker.
        assert!(
            has(&d, &s, "http://ex/marker", "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", "http://ex/Proven"),
            "backward rule projects a list-valued binding onto the goal variable"
        );
    }

    #[test]
    fn backward_chain_three_steps_within_depth() {
        // A 3-link backward chain (transitive ancestor) proven goal-directed: each recursive
        // link consumes one of the REMAINING depth budget (`depth - 1`), exercising the
        // depth-decrement path distinctly from the cyclic-cutoff test. Finite, well within
        // the backward depth limit.
        let src = r#"
            @prefix : <http://ex/> .
            :a :parent :b . :b :parent :c . :c :parent :d .
            { ?x :ancestor ?y } <= { ?x :parent ?y } .
            { ?x :ancestor ?y } <= { ?x :parent ?z . ?z :ancestor ?y } .
            { :a :ancestor :d } => { :a :reaches :d } .
        "#;
        let (d, s) = closure(src);
        // a→b→c→d: a is an ancestor of d only via THREE recursive backward applications.
        assert!(
            has(&d, &s, "http://ex/a", "http://ex/reaches", "http://ex/d"),
            "transitive ancestor proven through a 3-deep backward recursion"
        );
    }

    #[test]
    fn backward_premise_reads_fact_store_list() {
        // The backward rule's premise contains a builtin (list:member) over a list that lives
        // in the FACT STORE (asserted as rdf:first/rest data, reached through a bound var) —
        // driving `fact_list`. The goal binds ?item from the data collection.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix list: <http://www.w3.org/2000/10/swap/list#> .
            :bag :holds ( :apple :pear :plum ) .
            { :bag :contains ?item } <= { :bag :holds ?l . ?l list:member ?item } .
            { :bag :contains :pear } => { :pear a :Found } .
        "#;
        let (d, s) = closure(src);
        assert!(
            has(&d, &s, "http://ex/pear", "http://www.w3.org/1999/02/22-rdf-syntax-ns#type", "http://ex/Found"),
            "backward premise iterates a fact-store list to prove membership"
        );
    }

    #[test]
    fn math_quotient_double_division_propagates_inf_nan() {
        // cwm math/inf.n3 test5: under DOUBLE arithmetic, division by zero does NOT fail — it
        // yields IEEE ±INF (and 0.0/0.0 → NaN), which PROPAGATE as xsd:double literals. The
        // exact-arithmetic path (`b == 0 && !any_double`) would instead fail the premise; the
        // e-notation operands force `any_double`, taking the IEEE branch.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :seed :p :o .
            { ( 1.0e0 0.0e0 ) math:quotient ?pos } => { :s :posInf ?pos } .
            { ( -1.0e0 0.0e0 ) math:quotient ?neg } => { :s :negInf ?neg } .
            { ( 0.0e0 0.0e0 ) math:quotient ?nan } => { :s :nan ?nan } .
        "#;
        let (mut d, s) = closure(src);
        let dbl = "http://www.w3.org/2001/XMLSchema#double";
        let st = id(&d, "http://ex/s");
        let pinf = d.intern_lit("INF", dbl, None);
        let ninf = d.intern_lit("-INF", dbl, None);
        let nan = d.intern_lit("NaN", dbl, None);
        assert!(s.contains(&[st, id(&d, "http://ex/posInf"), pinf]), "1.0/0.0 → INF (double)");
        assert!(s.contains(&[st, id(&d, "http://ex/negInf"), ninf]), "-1.0/0.0 → -INF (double)");
        assert!(s.contains(&[st, id(&d, "http://ex/nan"), nan]), "0.0/0.0 → NaN propagates (not premise failure)");
    }

    #[test]
    fn math_inf_input_propagates_through_sum() {
        // An INF-valued (xsd:double) input propagates through arithmetic without tripping the
        // NaN domain-error guard: INF + 1 = INF (the guard only fires when a NaN result arises
        // from FINITE inputs). Pins the `!nums.iter().any(|x| x.is_nan() || x.is_infinite())`
        // exemption that lets INF/NaN inputs flow through.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :seed :p :o .
            { ( "INF"^^<http://www.w3.org/2001/XMLSchema#double> 1.0e0 ) math:sum ?x }
                => { :s :infPlus ?x } .
        "#;
        let (mut d, s) = closure(src);
        let dbl = "http://www.w3.org/2001/XMLSchema#double";
        let inf = d.intern_lit("INF", dbl, None);
        assert!(
            s.contains(&[id(&d, "http://ex/s"), id(&d, "http://ex/infPlus"), inf]),
            "INF + 1 = INF (infinite input propagates, no domain-error)"
        );
    }

    #[test]
    fn math_finite_nan_result_fails_premise_not_propagated() {
        // The COMPLEMENT of propagation: a NaN arising from FINITE inputs (acos 2, outside
        // [-1,1]) is a domain error — the premise FAILS rather than emitting a NaN literal.
        // This pins the guard's positive case against the INF/NaN-exemption above.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix math: <http://www.w3.org/2000/10/swap/math#> .
            :seed :p :o .
            { 2.0e0 math:acos ?x } => { :s :bad ?x } .
        "#;
        let (d, s) = closure(src);
        assert!(
            !s.iter().any(|[a, p, _]| *a == id(&d, "http://ex/s") && *p == id(&d, "http://ex/bad")),
            "acos 2 (finite NaN) fails the premise — no NaN literal emitted"
        );
    }

    #[test]
    fn includes_virtual_first_rest_over_list() {
        // [OPUS-4.8] sq-fodw5: log:includes containment where the OBJECT pattern probes
        // `rdf:first`/`rdf:rest` of a LIST VALUE that lives in the scope (cwm builtins.n3
        // test2/4 shape). The list structure is virtual — there are no rdf:first/rest triples
        // in the scope — so `containment_search` must synthesise the head/tail from the
        // first-class list term. Drives the `Term::List` virtual-first/rest branch.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            :seed :p :o .
            { { :s :list ( :a :b :c ) }
              log:includes
              { :s :list ?l . ?l rdf:first ?h . ?l rdf:rest ?t . ?t rdf:first ?h2 } }
            => { ?h a :Head . ?h2 a :Second } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        // head = first member :a ; rest's head = :b — virtual list walk inside containment.
        assert!(has(&d, &s, "http://ex/a", ty, "http://ex/Head"), "virtual rdf:first = list head");
        assert!(has(&d, &s, "http://ex/b", ty, "http://ex/Second"), "virtual rdf:rest then rdf:first = 2nd");
    }

    #[test]
    fn includes_virtual_first_deferred_binding() {
        // Same virtual-first/rest containment, but the list-bearing triple is written BEFORE
        // the triple that binds the list subject — so `containment_search` must DEFER the
        // first/rest pattern (rotate it behind the binding triple) and retry once `?l` is
        // bound. Drives the `Term::Var if defers_left > 0` rotation branch.
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            :seed :p :o .
            { { :s :list ( :x :y ) }
              log:includes
              { ?l rdf:first ?h . :s :list ?l } }
            => { ?h a :DeferredHead } .
        "#;
        let (d, s) = closure(src);
        let ty = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
        assert!(
            has(&d, &s, "http://ex/x", ty, "http://ex/DeferredHead"),
            "first/rest pattern deferred until the list subject binds, then resolved"
        );
    }

    #[test]
    fn forward_rule_existential_conclusion_skolemized_per_firing() {
        // [OPUS-4.8] sq-fodw5: a forward (`=>`) rule whose CONCLUSION introduces a blank node
        // (`[ … ]`) is an EXISTENTIAL — each distinct firing must mint a FRESH skolem, never
        // share one node across firings (cwm/EYE existential-introduction semantics). Drives
        // the per-firing skolemization block (the `__sk{n}_` rename keyed on the firing's
        // distinct binding). Two :Person individuals ⇒ two distinct :Parent blanks.
        let src = r#"
            @prefix : <http://ex/> .
            :alice a :Person . :bob a :Person .
            { ?p a :Person } => { ?p :hasParent [ a :Parent ] } .
        "#;
        let mut dict = Dict::new();
        let triples = reason_n3(&mut dict, src).unwrap();
        let hp = dict.intern_iri("http://ex/hasParent");
        let ty = dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let parent = dict.intern_iri("http://ex/Parent");
        let alice = dict.intern_iri("http://ex/alice");
        let bob = dict.intern_iri("http://ex/bob");

        // alice and bob each get a :hasParent edge to a node typed :Parent.
        let parent_of = |who: Id| -> Option<Id> {
            triples.iter().find(|[s, p, _]| *s == who && *p == hp).map(|[_, _, o]| *o)
        };
        let (ap, bp) = (parent_of(alice).expect("alice has a parent"), parent_of(bob).expect("bob has a parent"));
        assert!(triples.contains(&[ap, ty, parent]), "alice's parent is typed :Parent");
        assert!(triples.contains(&[bp, ty, parent]), "bob's parent is typed :Parent");
        // The load-bearing existential invariant: the two firings mint DISTINCT blanks.
        assert_ne!(ap, bp, "each rule firing mints a fresh existential, not a shared node");
        // Exactly two :Parent existentials exist (no spurious extra / no collapse to one).
        let n_parents = triples.iter().filter(|[_, p, o]| *p == ty && *o == parent).count();
        assert_eq!(n_parents, 2, "one fresh :Parent per firing — two firings, two parents");
    }

    #[test]
    fn forward_rule_existential_avoids_source_blank_label_collision() {
        // [SONNET-4.6] Issue #3496: the first minted `_:e` used to become
        // `_:__sk1_e`, conflating it with the source node carrying that label.
        let src = r#"
            @prefix : <http://ex/> .
            _:__sk1_e a :Source .
            :alice a :Person .
            { ?p a :Person } => { ?p :hasParent _:e . _:e a :Parent } .
        "#;
        let mut dict = Dict::new();
        let triples = reason_n3(&mut dict, src).unwrap();
        let ty = dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let source = dict.intern_iri("http://ex/Source");
        let parent = dict.intern_iri("http://ex/Parent");
        let has_parent = dict.intern_iri("http://ex/hasParent");
        let alice = dict.intern_iri("http://ex/alice");
        let source_node =
            triples.iter().find(|[_, p, o]| *p == ty && *o == source).unwrap()[0];
        let parent_node =
            triples.iter().find(|[_, p, o]| *p == ty && *o == parent).unwrap()[0];
        let parent_object =
            triples.iter().find(|[s, p, _]| *s == alice && *p == has_parent).unwrap()[2];

        assert_ne!(
            source_node, parent_node,
            "a minted rule existential must not capture a source blank label"
        );
        assert_eq!(
            parent_node, parent_object,
            "the existential must co-refer across its :hasParent and rdf:type triples"
        );
    }

    // ---- [OPUS-4.8] sq-qcnn.16: coverage gap tests for uncovered builtin paths ----

    /// `time:hours`, `time:minutes`, `time:seconds` — the H:M:S arm of `datetime_part`.
    /// The existing `time_components_and_unary_math` test only exercises year/month/day.
    #[test]
    fn time_hours_minutes_seconds() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .
            :e :when "2024-03-15T10:30:45"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
            { ?x :when ?d .
              ?d time:hours ?h .
              ?d time:minutes ?mi .
              ?d time:seconds ?s }
            => { ?x :h ?h ; :mi ?mi ; :s ?s } .
        "#;
        let (mut d, s) = closure(src);
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let e = id(&d, "http://ex/e");
        assert!(s.contains(&[e, id(&d, "http://ex/h"),  d.intern_lit("10", int, None)]), "time:hours = 10");
        assert!(s.contains(&[e, id(&d, "http://ex/mi"), d.intern_lit("30", int, None)]), "time:minutes = 30");
        assert!(s.contains(&[e, id(&d, "http://ex/s"),  d.intern_lit("45", int, None)]), "time:seconds = 45");
    }

    /// `time:dayOfWeek` and `time:inSeconds` (forward epoch encoding) — exercise the
    /// `DayOfWeek | InSeconds` arm of `datetime_part` and the `days_from_civil` helper.
    /// 1970-01-01 = day 0 (epoch), a Thursday (cwm: 4).
    #[test]
    fn time_day_of_week_and_in_seconds_forward() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .
            :e :when "1970-01-01T00:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
            { ?x :when ?d . ?d time:dayOfWeek ?dow . ?d time:inSeconds ?secs }
            => { ?x :dow ?dow ; :secs ?secs } .
        "#;
        let (mut d, s) = closure(src);
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let e = id(&d, "http://ex/e");
        // 1970-01-01 is epoch second 0, and a Thursday = day-of-week 4 (cwm indexing).
        assert!(s.contains(&[e, id(&d, "http://ex/secs"), d.intern_lit("0", int, None)]),
            "1970-01-01T00:00:00Z = epoch 0");
        assert!(s.contains(&[e, id(&d, "http://ex/dow"), d.intern_lit("4", int, None)]),
            "1970-01-01 is Thursday (dayOfWeek=4 in cwm indexing)");
    }

    /// `time:inSeconds` **reverse** mode: an epoch integer → UTC dateTime string.
    /// Drives the `Func::InSeconds` arm inside `eval_functional`'s reverse-mode block.
    #[test]
    fn time_in_seconds_reverse() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .
            :seed :p :o .
            { ?dt time:inSeconds 0 } => { :r :dt ?dt } .
        "#;
        let (mut d, s) = closure(src);
        let xsd_str = "http://www.w3.org/2001/XMLSchema#string";
        let epoch = d.intern_lit("1970-01-01T00:00:00Z", xsd_str, None);
        assert!(s.contains(&[id(&d, "http://ex/r"), id(&d, "http://ex/dt"), epoch]),
            "reverse time:inSeconds 0 = 1970-01-01T00:00:00Z");
    }

    /// #3804 — `time:inSeconds` / `time:dayOfWeek` must DECLINE (bind nothing) on an
    /// impossible calendar date instead of silently rolling it over (2026-02-29 used
    /// to become 2026-03-01), matching XSD and `sparq_policy`'s `parse_instant`. A real
    /// leap day (2024-02-29) still binds.
    #[test]
    fn time_in_seconds_declines_impossible_calendar_dates() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            :bad1 :when "2026-02-29T00:00:00Z"^^xsd:dateTime .
            :bad2 :when "2026-04-31T00:00:00Z"^^xsd:dateTime .
            :bad3 :when "2026-13-01T00:00:00Z"^^xsd:dateTime .
            :bad4 :when "2026-01-00T00:00:00Z"^^xsd:dateTime .
            :bad5 :when "1900-02-29T00:00:00Z"^^xsd:dateTime .
            :ok1 :when "2024-02-29T00:00:00Z"^^xsd:dateTime .
            :ok2 :when "2000-02-29T00:00:00Z"^^xsd:dateTime .
            { ?x :when ?d . ?d time:inSeconds ?secs } => { ?x :secs ?secs } .
            { ?x :when ?d . ?d time:dayOfWeek ?dow } => { ?x :dow ?dow } .
        "#;
        let (d, s) = closure(src);
        let secs = id(&d, "http://ex/secs");
        let dow = id(&d, "http://ex/dow");
        for bad in ["bad1", "bad2", "bad3", "bad4", "bad5"] {
            let e = id(&d, &format!("http://ex/{bad}"));
            assert!(
                !s.iter().any(|t| t[0] == e && (t[1] == secs || t[1] == dow)),
                "{bad}: an impossible calendar date must not bind time:inSeconds/dayOfWeek"
            );
        }
        for ok in ["ok1", "ok2"] {
            let e = id(&d, &format!("http://ex/{ok}"));
            assert!(
                s.iter().any(|t| t[0] == e && t[1] == secs),
                "{ok}: leap day must bind"
            );
            assert!(
                s.iter().any(|t| t[0] == e && t[1] == dow),
                "{ok}: leap day must bind"
            );
        }
    }

    /// A supplied-but-unparsable date field must decline, not fall back to the
    /// cwm default for an ABSENT component: `"2026-02-29Z"^^xsd:date` used to
    /// read its day `29Z` as a parse failure → day 1 (a valid Feb 1), and an
    /// overflowing day did the same. A date's timezone suffix is separated before
    /// the calendar fields are parsed, so valid zoned dates still bind correctly.
    #[test]
    fn time_builtins_decline_unparsable_supplied_date_fields() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            :bad1 :when "2026-02-29Z"^^xsd:date .
            :bad2 :when "2026-02-9223372036854775808T00:00:00Z"^^xsd:dateTime .
            :bad3 :when "2026-02-29+05:00"^^xsd:date .
            :bad4 :when "2026-0x-01T00:00:00Z"^^xsd:dateTime .
            :bad5 :when "2026-03-01Tab:00:00Z"^^xsd:dateTime .
            :bad6 :when "9223372036854775807-01-01T00:00:00Z"^^xsd:dateTime .
            :ok1 :when "2026-03-15Z"^^xsd:date .
            :ok2 :when "2026-03-15-05:00"^^xsd:date .
            :ok3 :when "2026-03-15"^^xsd:date .
            :ok4 :when "2026"^^xsd:gYear .
            { ?x :when ?d . ?d time:inSeconds ?secs } => { ?x :secs ?secs } .
            { ?x :when ?d . ?d time:dayOfWeek ?dow } => { ?x :dow ?dow } .
        "#;
        let (mut d, s) = closure(src);
        let secs = id(&d, "http://ex/secs");
        let dow = id(&d, "http://ex/dow");
        for bad in ["bad1", "bad2", "bad3", "bad4", "bad5", "bad6"] {
            let e = id(&d, &format!("http://ex/{bad}"));
            assert!(
                !s.iter().any(|t| t[0] == e && (t[1] == secs || t[1] == dow)),
                "{bad}: an unparsable supplied date field must not bind time builtins"
            );
        }
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        for (ok, want) in [
            ("ok1", "1773532800"), // 2026-03-15T00:00:00Z
            ("ok3", "1773532800"),
            ("ok2", "1773550800"), // midnight at -05:00 is 05:00Z
            ("ok4", "1767225600"), // bare year: ABSENT month/day default to 01-01
        ] {
            let e = id(&d, &format!("http://ex/{ok}"));
            let lit = d.intern_lit(want, int, None);
            assert!(s.contains(&[e, secs, lit]), "{ok}: time:inSeconds must be {want}");
        }
        for ok in ["ok1", "ok2", "ok3"] {
            let e = id(&d, &format!("http://ex/{ok}"));
            assert!(s.iter().any(|t| t[0] == e && t[1] == dow), "{ok}: must bind dayOfWeek");
        }
    }

    /// `time:timeZone` — extracts the explicit ±hh:mm timezone offset from a dateTime.
    /// Drives the `Func::TimeZone` branch of `eval_functional`.
    #[test]
    fn time_timezone_explicit_offset() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix time: <http://www.w3.org/2000/10/swap/time#> .
            :e :when "2024-03-15T10:30:45+05:30"^^<http://www.w3.org/2001/XMLSchema#dateTime> .
            { ?x :when ?d . ?d time:timeZone ?tz } => { ?x :tz ?tz } .
        "#;
        let (mut d, s) = closure(src);
        let xsd_str = "http://www.w3.org/2001/XMLSchema#string";
        let tz = d.intern_lit("+05:30", xsd_str, None);
        let e = id(&d, "http://ex/e");
        assert!(s.contains(&[e, id(&d, "http://ex/tz"), tz]),
            "time:timeZone = +05:30 from explicit offset");
    }

    /// `log:langlit` — constructs a language-tagged literal from (lexical-form, lang-tag).
    /// Drives the `Func::Langlit` branch of `eval_functional`.
    #[test]
    fn log_langlit_construct() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix log: <http://www.w3.org/2000/10/swap/log#> .
            :seed :p :o .
            { ("Hola" "es") log:langlit ?lit } => { :r :greeting ?lit } .
        "#;
        let (mut d, s) = closure(src);
        let lang_string = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
        let greeting = d.intern_lit("Hola", lang_string, Some("es"));
        let r = id(&d, "http://ex/r");
        assert!(s.contains(&[r, id(&d, "http://ex/greeting"), greeting]),
            "log:langlit ( \"Hola\" \"es\" ) = \"Hola\"@es");
    }

    /// `string:encodeForURI` (cwm variant, keeps #'()~ but encodes /) and
    /// `string:encodeForFragID` (keeps / but encodes #'()~). Both drive the
    /// `EncodeForUriCwm | EncodeForFragId` branch of `eval_functional`.
    #[test]
    fn string_encode_for_uri_cwm_and_fragid() {
        let src = r#"
            @prefix : <http://ex/> .
            @prefix string: <http://www.w3.org/2000/10/swap/string#> .
            :seed :p :o .
            { "hello/world#test" string:encodeForURI ?cwm } => { :r :cwm ?cwm } .
            { "hello/world#test" string:encodeForFragID ?frag } => { :r :frag ?frag } .
        "#;
        let (mut d, s) = closure(src);
        let xsd_str = "http://www.w3.org/2001/XMLSchema#string";
        let r = id(&d, "http://ex/r");
        // cwm keeps #'()~ but encodes /; so "hello/world#test" → "hello%2Fworld#test"
        let cwm_enc = d.intern_lit("hello%2Fworld#test", xsd_str, None);
        assert!(s.contains(&[r, id(&d, "http://ex/cwm"), cwm_enc]),
            "string:encodeForURI (cwm) encodes / but keeps #");
        // fragID keeps / but encodes #; so "hello/world#test" → "hello/world%23test"
        let frag_enc = d.intern_lit("hello/world%23test", xsd_str, None);
        assert!(s.contains(&[r, id(&d, "http://ex/frag"), frag_enc]),
            "string:encodeForFragID keeps / but encodes #");
    }
}

/// [OPUS-4.8] sq-pbz04.5.1 — DIRECT differential over the seam-2 tower adoption: the
/// current (substrate-`Dec`-backed) exact-arithmetic core vs a verbatim re-derivation of
/// the OLD private-`NumVal` semantics, plus pinned EYE-edge outputs. The two must agree
/// BYTE-FOR-BYTE (`Term` equality — lexical + datatype). Covers the arithmetic matrix the
/// bead names: `> i64::MAX` integers (the i128↔i64 wrinkle), exact-decimal add/sub/mul
/// (`0.1 + 0.2` = `0.3`), INF/NaN, and the `('2.7' '2') math:difference = 0.7` exactness
/// case. Each assertion pins an EXACT value so a mutation of the arithmetic / rendering /
/// branch logic goes red.
#[cfg(test)]
mod substrate_seam_differential {
    /// The value of [`eval_exact`], settled into a throwaway sink.
    fn exact_value(f: Func, args: &[Term]) -> Option<Term> {
        super::bounded::settle(&std::cell::Cell::new(false), eval_exact(f, args)).map(super::numval_term)
    }

    /// Whether [`eval_exact`] marks a cut.
    fn exact_cut(f: Func, args: &[Term]) -> bool {
        let sink = std::cell::Cell::new(false);
        let _ = super::bounded::settle(&sink, eval_exact(f, args));
        sink.get()
    }

    #[test]
    fn overflow_is_a_cut_and_a_representable_result_is_not() {
        let min = i128::MIN.to_string();
        let max = i128::MAX.to_string();
        // i128::MIN % -1 is exactly 0: computed, no cut.
        assert_eq!(
            exact_value(Func::Remainder, &[plain(&min), plain("-1")]),
            Some(lit("0", XSD_INT))
        );
        assert!(!exact_cut(Func::Remainder, &[plain(&min), plain("-1")]));
        // Results past i128 are cuts.
        for (f, args) in [
            (Func::Sum, vec![plain(&max), plain("1")]),
            (Func::Difference, vec![plain(&min), plain("1")]),
            (Func::Product, vec![plain(&max), plain("2")]),
            (Func::Quotient, vec![plain(&min), plain("-1")]),
            (Func::IntegerQuotient, vec![plain(&min), plain("-1")]),
            (Func::Negation, vec![plain(&min)]),
            (Func::AbsoluteValue, vec![plain(&min)]),
            (Func::Exponentiation, vec![plain("2"), plain("200")]),
            (Func::Sum, vec![plain("1"), plain(&format!("{max}0"))]),
        ] {
            assert!(exact_cut(f, &args), "{:?} {:?} must be a cut", f, args);
        }
        // A non-terminating quotient is EYE's double result, not a cut.
        assert!(!exact_cut(Func::Quotient, &[plain("1"), plain("3")]));
    }

    use super::*;

    const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
    const XSD_DEC: &str = "http://www.w3.org/2001/XMLSchema#decimal";
    const XSD_DBL: &str = "http://www.w3.org/2001/XMLSchema#double";

    fn lit(v: &str, dt: &str) -> Term {
        Term::Lit(v.to_string(), dt.into(), None)
    }
    fn plain(v: &str) -> Term {
        // A plain (un-typed) literal — the chainer coerces it by LEXICAL SHAPE via `numval`.
        Term::Lit(v.to_string(), XSD_INT.into(), None)
    }

    // --- The OLD private-`NumVal` exact core, re-derived VERBATIM as the differential
    // oracle (the pre-adoption code the seam replaced). It must produce the identical
    // `(mant, scale)` the current substrate-`Dec`-backed core does for the delegated ops
    // (Sum/Product/Difference/Max/Min/Negation/AbsoluteValue). ---

    #[derive(Clone, Copy)]
    enum OldNum {
        Int(i128),
        Dec(i128, u32),
        // The old tower's `F64` tier: `eval_exact` returns `None` on ANY `F64` input (the
        // exact core is integer/decimal-only), so the payload is never consulted — a unit
        // marker is enough for the oracle and avoids a dead-field lint. [OPUS-4.8]
        F64,
    }
    fn old_parse(t: &Term) -> Option<OldNum> {
        match numval(t)? {
            NumVal::Int(i) => Some(OldNum::Int(i)),
            NumVal::Dec(m, s) => Some(OldNum::Dec(m, s)),
            NumVal::F64(_) => Some(OldNum::F64),
        }
    }
    fn old_aligned(a: OldNum, b: OldNum) -> Option<(i128, i128, u32)> {
        let part = |v: OldNum| match v {
            OldNum::Int(i) => Some((i, 0u32)),
            OldNum::Dec(m, s) => Some((m, s)),
            OldNum::F64 => None,
        };
        let ((ma, sa), (mb, sb)) = (part(a)?, part(b)?);
        let s = sa.max(sb);
        let up = |m: i128, from: u32| -> Option<i128> { m.checked_mul(10i128.checked_pow(s - from)?) };
        Some((up(ma, sa)?, up(mb, sb)?, s))
    }
    /// The pre-adoption `eval_exact` for the DELEGATED arithmetic ops (verbatim i128 algorithm).
    fn old_eval_exact(f: Func, args: &[Term]) -> Option<Term> {
        let vals: Vec<OldNum> = args.iter().map(old_parse).collect::<Option<_>>()?;
        if vals.iter().any(|v| matches!(v, OldNum::F64)) {
            return None;
        }
        let any_dec = vals.iter().any(|v| matches!(v, OldNum::Dec(_, _)));
        let renorm = |m: i128, s: u32| if any_dec { NumVal::Dec(m, s) } else { NumVal::Int(m) };
        let out = match f {
            Func::Sum => {
                let mut acc = OldNum::Int(0);
                for &v in &vals {
                    let (a, b, s) = old_aligned(acc, v)?;
                    acc = OldNum::Dec(a.checked_add(b)?, s);
                }
                let OldNum::Dec(m, s) = acc else { return None };
                renorm(m, s)
            }
            Func::Product => {
                let mut acc = OldNum::Int(1);
                for &v in &vals {
                    let (ma, sa) = match acc {
                        OldNum::Int(i) => (i, 0),
                        OldNum::Dec(m, s) => (m, s),
                        OldNum::F64 => return None,
                    };
                    let (mb, sb) = match v {
                        OldNum::Int(i) => (i, 0),
                        OldNum::Dec(m, s) => (m, s),
                        OldNum::F64 => return None,
                    };
                    acc = OldNum::Dec(ma.checked_mul(mb)?, sa.checked_add(sb)?);
                }
                let OldNum::Dec(m, s) = acc else { return None };
                renorm(m, s)
            }
            Func::Difference => {
                if vals.len() != 2 {
                    return None;
                }
                let (a, b, s) = old_aligned(vals[0], vals[1])?;
                renorm(a.checked_sub(b)?, s)
            }
            Func::Max | Func::Min => {
                let mut best = vals[0];
                for &v in &vals[1..] {
                    let (a, b, _) = old_aligned(best, v)?;
                    let take = if matches!(f, Func::Max) { b > a } else { b < a };
                    if take {
                        best = v;
                    }
                }
                match best {
                    OldNum::Int(i) => NumVal::Int(i),
                    OldNum::Dec(m, s) => NumVal::Dec(m, s),
                    OldNum::F64 => return None,
                }
            }
            Func::Negation => match vals[0] {
                OldNum::Int(i) => NumVal::Int(i.checked_neg()?),
                OldNum::Dec(m, s) => NumVal::Dec(m.checked_neg()?, s),
                OldNum::F64 => return None,
            },
            Func::AbsoluteValue => match vals[0] {
                OldNum::Int(i) => NumVal::Int(i.checked_abs()?),
                OldNum::Dec(m, s) => NumVal::Dec(m.checked_abs()?, s),
                OldNum::F64 => return None,
            },
            _ => return None,
        };
        Some(numval_term(out))
    }

    /// The differential invariant: for the DELEGATED ops the substrate-backed `eval_exact`
    /// equals the old-semantics oracle byte-for-byte.
    fn assert_diff(f: Func, args: &[Term]) {
        assert_eq!(
            exact_value(f, args),
            old_eval_exact(f, args),
            "substrate-backed eval_exact diverged from the old NumVal semantics for {:?} on {:?}",
            f as u8,
            args
        );
    }

    #[test]
    fn diff_sum_difference_product_over_matrix() {
        // Integer, decimal, mixed, multi-arg — the value+scale-preserving delegated ops.
        let cases: &[(Func, Vec<Term>)] = &[
            (Func::Sum, vec![plain("2"), plain("3")]),
            (Func::Sum, vec![lit("0.1", XSD_DEC), lit("0.2", XSD_DEC)]),
            (Func::Sum, vec![plain("1"), lit("0.20", XSD_DEC)]),
            (Func::Sum, vec![plain("1"), plain("2"), plain("3"), lit("0.5", XSD_DEC)]),
            (Func::Difference, vec![lit("2.7", XSD_DEC), plain("2")]),
            (Func::Difference, vec![plain("10"), plain("3")]),
            (Func::Product, vec![lit("2.7", XSD_DEC), lit("2.7", XSD_DEC)]),
            (Func::Product, vec![plain("6"), plain("7")]),
            (Func::Product, vec![lit("0.1", XSD_DEC), lit("0.2", XSD_DEC), plain("2")]),
            (Func::Max, vec![plain("3"), lit("2.9", XSD_DEC), plain("5")]),
            (Func::Min, vec![lit("2.7", XSD_DEC), plain("2"), lit("2.71", XSD_DEC)]),
            (Func::Negation, vec![lit("3.50", XSD_DEC)]),
            (Func::AbsoluteValue, vec![lit("-3.50", XSD_DEC)]),
        ];
        for (f, args) in cases {
            assert_diff(*f, args);
        }
    }

    #[test]
    fn diff_exact_decimal_add_sub_mul_pinned() {
        // 0.1 + 0.2 is EXACTLY 0.3 (the f64 path gets 0.30000000000000004).
        assert_eq!(
            exact_value(Func::Sum, &[lit("0.1", XSD_DEC), lit("0.2", XSD_DEC)]),
            Some(lit("0.3", XSD_DEC))
        );
        // ('2.7' '2') math:difference = 0.7 EXACTLY (f64 gives 0.7000000000000002).
        assert_eq!(
            exact_value(Func::Difference, &[lit("2.7", XSD_DEC), plain("2")]),
            Some(lit("0.7", XSD_DEC))
        );
        // 2.7 * 2.7 = 7.29 exactly (scale 1 * scale 1 → scale 2).
        assert_eq!(
            exact_value(Func::Product, &[lit("2.7", XSD_DEC), lit("2.7", XSD_DEC)]),
            Some(lit("7.29", XSD_DEC))
        );
        // A trailing-zero scale is normalised by numval_term/dec_norm: 1 + 0.20 → "1.2".
        assert_eq!(
            exact_value(Func::Sum, &[plain("1"), lit("0.20", XSD_DEC)]),
            Some(lit("1.2", XSD_DEC))
        );
    }

    #[test]
    fn diff_over_i64_max_integers_i128_wrinkle() {
        // The i128↔i64 wrinkle: chainer Int is i128, substrate Num::Int is i64, so an
        // out-of-i64 integer is carried as substrate Dec { mant, scale: 0 } (EXACT). The
        // sum stays an xsd:integer and is EXACT (no f64 collapse).
        let a = (i64::MAX as i128) + 1; // 9223372036854775808, beyond i64
        let b: i128 = 1000;
        let sum = a + b;
        assert_eq!(
            exact_value(Func::Sum, &[plain(&a.to_string()), plain(&b.to_string())]),
            Some(lit(&sum.to_string(), XSD_INT)),
            "sum of a > i64::MAX integer stays an exact xsd:integer via the substrate Dec carrier"
        );
        // Product of two large integers, still exact within i128.
        let big = 3_037_000_500i128; // ~sqrt(i128::MAX)/... well within i128 when squared? no — pick safe
        let sq = big.checked_mul(big).unwrap();
        assert_eq!(
            exact_value(
                Func::Product,
                &[plain(&big.to_string()), plain(&big.to_string())]
            ),
            Some(lit(&sq.to_string(), XSD_INT))
        );
        // Difference crossing the i64 boundary.
        let hi = (i64::MAX as i128) + 500;
        assert_eq!(
            exact_value(Func::Difference, &[plain(&hi.to_string()), plain("500")]),
            Some(lit(&(i64::MAX).to_string(), XSD_INT))
        );
        // All three delegated ops match the old oracle on the >i64 matrix.
        assert_diff(Func::Sum, &[plain(&a.to_string()), plain(&b.to_string())]);
        assert_diff(Func::Product, &[plain(&big.to_string()), plain(&big.to_string())]);
        assert_diff(Func::Difference, &[plain(&hi.to_string()), plain("500")]);
    }

    #[test]
    fn diff_inf_nan_stay_on_f64_fallback() {
        // INF / NaN have no exact tier: eval_exact returns None (→ the f64 fallback path),
        // identically in both the substrate-backed and old cores.
        for special in ["INF", "-INF", "NaN"] {
            let args = [lit(special, XSD_DBL), plain("2")];
            assert_eq!(
                exact_value(Func::Sum, &args),
                None,
                "{} has no exact tier",
                special
            );
            assert_eq!(old_eval_exact(Func::Sum, &args), None);
        }
        // numval classifies the specials as F64 (the lexical-shape coercion edge).
        assert!(matches!(numval(&lit("INF", XSD_DBL)), Some(NumVal::F64(f)) if f == f64::INFINITY));
        assert!(matches!(numval(&lit("-INF", XSD_DBL)), Some(NumVal::F64(f)) if f == f64::NEG_INFINITY));
        assert!(matches!(numval(&lit("NaN", XSD_DBL)), Some(NumVal::F64(f)) if f.is_nan()));
    }

    #[test]
    fn diff_kept_eye_ops_unchanged() {
        // The EYE-specific ops that KEEP their algorithm (quotient's scale-34 / type rule,
        // remainder's divisor-sign, integer-quotient's floor) are pinned to their EYE output
        // so the refactor cannot have perturbed them.
        // integer / integer exact → xsd:integer (NOT a "N.0" decimal — the seam declines
        // substrate Dec::checked_div here, which would always yield a decimal).
        assert_eq!(
            exact_value(Func::Quotient, &[plain("6"), plain("2")]),
            Some(lit("3", XSD_INT))
        );
        // exact terminating quotient → decimal.
        assert_eq!(
            exact_value(Func::Quotient, &[plain("1"), plain("4")]),
            Some(lit("0.25", XSD_DEC))
        );
        // non-terminating → None (f64 fallback), NOT a rounded decimal.
        assert_eq!(exact_value(Func::Quotient, &[plain("1"), plain("3")]), None);
        // remainder: divisor-sign (Python %): -2 mod 4 = 2, 2 mod -4 = -2.
        assert_eq!(
            exact_value(Func::Remainder, &[plain("-2"), plain("4")]),
            Some(lit("2", XSD_INT))
        );
        assert_eq!(
            exact_value(Func::Remainder, &[plain("2"), plain("-4")]),
            Some(lit("-2", XSD_INT))
        );
        // integerQuotient: floor division.
        assert_eq!(
            exact_value(Func::IntegerQuotient, &[plain("-7"), plain("2")]),
            Some(lit("-4", XSD_INT))
        );
        // floor/ceiling collapse a decimal to an xsd:integer; rounded keeps the "N.0" decimal.
        assert_eq!(
            exact_value(Func::Floor, &[lit("2.7", XSD_DEC)]),
            Some(lit("2", XSD_INT))
        );
        assert_eq!(
            exact_value(Func::Ceiling, &[lit("2.1", XSD_DEC)]),
            Some(lit("3", XSD_INT))
        );
        assert_eq!(
            exact_value(Func::Rounded, &[lit("2.5", XSD_DEC)]),
            Some(lit("3.0", XSD_DEC))
        );
    }

    #[test]
    fn diff_negation_abs_tier_preserving() {
        // Negation / abs preserve the tier and scale (Dec stays Dec with its scale).
        assert_eq!(
            exact_value(Func::Negation, &[plain("5")]),
            Some(lit("-5", XSD_INT))
        );
        assert_eq!(
            exact_value(Func::Negation, &[lit("3.50", XSD_DEC)]),
            Some(lit("-3.5", XSD_DEC))
        );
        assert_eq!(
            exact_value(Func::AbsoluteValue, &[lit("-3.50", XSD_DEC)]),
            Some(lit("3.5", XSD_DEC))
        );
        // A > i64 integer negates exactly (i128 checked_neg on the substrate Dec carrier).
        let big = (i64::MAX as i128) + 7;
        assert_eq!(
            exact_value(Func::Negation, &[plain(&big.to_string())]),
            Some(lit(&(-big).to_string(), XSD_INT))
        );
    }
}

#[cfg(test)]
mod runtime_docs_table {
    /// A run that parses more distinct documents at run time than its table may hold fails
    /// with the runtime-document error and records it as a cut; with room it closes.
    #[test]
    fn a_run_past_the_runtime_document_table_fails_closed() {
        let src = "@prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
                   <http://ex/a> <http://ex/t> \"_:x <http://ex/p> 1 .\" .\n\
                   <http://ex/b> <http://ex/t> \"_:x <http://ex/p> 2 .\" .\n\
                   { ?d <http://ex/t> ?t . ?t log:parsedAsN3 ?f . \
                     ?f log:includes { ?s <http://ex/p> ?n } } => { ?d <http://ex/n> ?n } .";
        let run = |room: usize| {
            let _room = super::bounded::RuntimeDocsOverride::set(room);
            let limits = super::ClosureLimits::default();
            super::reason_n3_terms_with_limits(src, None, None, Default::default(), limits)
        };
        assert_eq!(run(2).expect("closes").derived.len(), 2);
        let Err(e) = run(1) else { panic!("a second document was taken by a table of one") };
        assert!(e.contains("runtime-document limit exceeded"), "{e}");
    }

    /// Standardizing a runtime document's blanks apart lengthens each label, and that
    /// growth is charged before any label is rewritten: a document that fits
    /// `max_term_bytes` as parsed but not once prefixed is refused, and one whose
    /// standardized formula is exactly the limit is kept.
    #[test]
    fn the_growth_of_standardized_blank_labels_is_charged_before_the_rewrite() {
        let text: String = (0..64).map(|i| format!("_:a{i} <http://ex/p> _:b{i} . ")).collect();
        let src = format!(
            "@prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             <http://ex/a> <http://ex/t> \"{text}\" .\n\
             {{ <http://ex/a> <http://ex/t> ?t . ?t log:parsedAsN3 ?f }} => \
             {{ <http://ex/a> <http://ex/out> ?f }} ."
        );
        let run = |max_term_bytes: usize| {
            let limits = super::ClosureLimits { max_term_bytes, ..Default::default() };
            super::reason_n3_terms_with_limits(&src, None, None, Default::default(), limits)
        };
        let closure = run(super::ClosureLimits::default().max_term_bytes).expect("closes");
        let [d] = closure.derived.as_slice() else { panic!("{:?}", closure.derived) };
        let super::Term::Formula(ts) = &d[2] else { panic!("{d:?}") };
        let standardized = super::bounded::term_bytes(&d[2]);
        let labels: Vec<&str> = ts
            .iter()
            .flatten()
            .filter_map(|t| match t {
                super::Term::Blank(l) => Some(l.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(labels.len(), 128);
        let prefix = labels[0].len() - "a0".len();
        assert!(labels[0].starts_with("__rp") && labels[0].ends_with("_a0"), "{}", labels[0]);
        let parsed = standardized - labels.len() * prefix;
        // Fits as parsed, not once standardized: refused, naming the builtin.
        let Err(e) = run(standardized - 1) else { panic!("kept a formula past the limit") };
        assert!(e.contains("max_term_bytes") && e.contains("ParsedAsN3"), "{e}");
        assert!(parsed < standardized);
        // Exactly the standardized size: kept, the same formula.
        let at = run(standardized).expect("a formula at the limit is kept");
        assert_eq!(at.derived, closure.derived);
    }
}
