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
//! `BW_DEPTH`), with structural unification through lists and quoted
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

/// Depth bound for goal-directed (`<=`) resolution: a backward proof may chain through at
/// most this many backward-rule applications. Bounds runaway recursion (e.g. a rule whose
/// premise re-poses its own goal) — within the bound, proofs are exhaustive.
const BW_DEPTH: usize = 64;

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
type VisitedDocs = std::rc::Rc<std::cell::RefCell<FxHashSet<u64>>>;

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
}

impl<'a> BwCtx<'a> {
    fn new(rules: &'a [Rule]) -> BwCtx<'a> {
        BwCtx {
            rules,
            rename: std::cell::Cell::new(0),
            base: String::new(),
            resolver: None,
            visited: VisitedDocs::default(),
        }
    }
}

/// One derivation step: a `conclusion` triple was produced by rule `rule` (its index in the
/// document's rule order) from the ground `premises` (the supporting facts under the binding;
/// premise atoms proven by backward rules are not themselves facts and are not listed).
pub struct ProofStep {
    pub conclusion: [Id; 3],
    pub rule: usize,
    pub premises: Vec<[Id; 3]>,
}

/// Parse N3 `src`, run the rule closure, and return the entailed GROUND triples interned into
/// `dict`. The rules/formulae/variables are consumed by reasoning; only ground facts remain.
pub fn reason_n3(dict: &mut Dict, src: &str) -> Result<Vec<[Id; 3]>, String> {
    // No derivation tracking ([`StepMode::None`]): skips per-firing premise materialization
    // in the hot loop and the proof-step interning pass entirely.
    let parsed = parser::parse(src)?;
    let (facts, steps) = run_closure(parsed, None, None, StepMode::None);
    Ok(intern_closure(dict, &facts, &steps)?.0)
}

/// As [`reason_n3`], but also return the derivation (a [`ProofStep`] for each NEWLY-derived
/// triple, in derivation order) — the EYE `--proof` analogue.
pub fn reason_n3_proof(dict: &mut Dict, src: &str) -> Result<(Vec<[Id; 3]>, Vec<ProofStep>), String> {
    let parsed = parser::parse(src)?;
    let (facts, steps) = run_closure(parsed, None, None, StepMode::Full);
    intern_closure(dict, &facts, &steps)
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
pub fn reason_n3_pass_all(src: &str, vars: RuleVars) -> Result<String, String> {
    let parsed = parser::parse(src)?;
    // Clone the rules BEFORE the closure runs: `run_closure` reorders each premise for
    // builtin readiness, and the echo should reflect the document, not that plan.
    let (rules, backward_rules) = (parsed.rules.clone(), parsed.backward_rules.clone());
    let (facts, _steps) = run_closure(parsed, None, None, StepMode::None);
    let mut statements: Vec<String> = facts
        .all
        .iter()
        .map(|f| {
            let mut s = String::new();
            serialize::write_statement(f, &mut s);
            s
        })
        .collect();
    statements.sort_unstable();
    let mut out = statements.concat();
    for r in &rules {
        serialize::write_rule(r, RuleKind::Forward, vars, &mut out);
    }
    for r in &backward_rules {
        serialize::write_rule(r, RuleKind::Backward, vars, &mut out);
    }
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
/// Errors when either document fails to parse, or when the query document has no forward rule
/// (a fact-only or backward-only query document has nothing to project — fail loudly rather
/// than return an empty answer that reads like "the query matched nothing").
pub fn reason_n3_query_terms(data: &str, query: &str) -> Result<Vec<[Term; 3]>, String> {
    let data_parsed = parser::parse(data)?;
    let query_parsed = parser::parse(query)?;
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
    let (facts, _steps) = run_closure(data_parsed, None, None, StepMode::None);
    let mut bw = BwCtx::new(&backward);
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
                if let Some(g) = ground_triple(&c, &b) {
                    if emitted.insert(g.clone()) {
                        out.push(g);
                    }
                }
            }
        }
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
    let answers = reason_n3_query_terms(data, query)?;
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
}

/// Run the rule closure STRATUM BY STRATUM: each `strata[i]` is a complete N3
/// document (facts + rules), and the full term-level closure of the strata
/// before it is carried forward IN MEMORY as additional input facts — no
/// serialize/re-parse round-trip between strata (formula-valued facts, which
/// a text round-trip cannot represent, carry over intact).
///
/// This is the sound driver for the engine's NON-MONOTONIC premise operators
/// (store-scoped `log:notIncludes`, `log:collectAllIn` / `log:forAllIn`):
/// those are only reliable over predicates FULLY PRESENT before their stratum
/// starts (rules containing them re-evaluate every fixpoint round but derived
/// facts are never retracted), so derive such predicates to a fixpoint in an
/// earlier stratum and negate/aggregate over them in a later one.
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
    let mut carried: Vec<[Term; 3]> = Vec::new();
    let mut facts = FactIndex::default();
    let mut strata_facts = Vec::with_capacity(strata.len());
    for (i, src) in strata.iter().enumerate() {
        let mut parsed = parser::parse(src)?;
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
        let (f, _steps) = run_closure(parsed, None, None, StepMode::None);
        strata_facts.push(f.all.len());
        if i + 1 < strata.len() {
            carried = f.all.iter().cloned().collect();
        }
        facts = f;
    }
    Ok(StratifiedN3Closure { facts: intern_closure(dict, &facts, &[])?.0, strata_facts })
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
) -> Result<(FxHashSet<[Term; 3]>, Vec<DerivationStep>), String> {
    let parsed = parser::parse(src)?;
    let (facts, steps) = run_closure(parsed, None, None, StepMode::Full);
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
}

/// As [`reason_n3`], but returns the closure at the TERM level (no dictionary)
/// and resolves relative IRIs against `base` when given — the entry point used
/// by the W3C N3 conformance harness (cwm/EYE-style: `--think` then compare).
pub fn reason_n3_terms(src: &str, base: Option<&str>) -> Result<N3Closure, String> {
    reason_n3_terms_with_resolver(src, base, None)
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
    let parsed = match base {
        Some(b) => parser::parse_with_base(src, b)?,
        None => parser::parse(src)?,
    };
    let (n_rules, n_backward_rules) = (parsed.rules.len(), parsed.backward_rules.len());
    // `derived` needs the conclusions in derivation order but never the premises.
    let (facts, steps) = run_closure(parsed, resolver, None, StepMode::Conclusions);
    Ok(N3Closure {
        facts: facts.all.into_iter().collect(),
        derived: steps.into_iter().map(|(g, _, _)| g).collect(),
        n_rules,
        n_backward_rules,
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

/// The semi-naive forward-chaining fixpoint shared by the id-level and
/// term-level entry points. Returns the final fact set plus the derivation
/// steps `(conclusion, rule index, supporting premises)` in derivation order
/// (as much of them as `mode` asks for).
fn run_closure(
    parsed: parser::Parsed,
    resolver: Option<&Resolver>,
    // The import-cycle guard ([`VisitedDocs`]). `None` for a TOP-LEVEL run (a fresh,
    // empty set); a nested run reached through `formula_closure` passes the PARENT's set so
    // a `log:semantics` / `log:content` document IRI active up the stack is still recognised.
    visited: Option<VisitedDocs>,
    mode: StepMode,
) -> (FactIndex, Vec<DerivationStep>) {
    // [SONNET-4.6] Rule existentials live in a namespace proven fresh against
    // every blank label in the parsed source, preventing a literal `_:__sk…`
    // from being captured by a minted conclusion blank. Labels ingested later
    // through document builtins or nested closure evaluation are outside this
    // initial-source guarantee.
    let sk_prefix = fresh_blank_prefix(&document_blank_labels(&parsed), "__sk");
    let parser::Parsed { facts: facts0, mut rules, mut backward_rules, base } = parsed;
    // Premises evaluate left-to-right with no coroutining — reorder each
    // premise so a builtin runs only after the atoms that produce its inputs
    // (cwm evaluates builtins "when ready"; concat.n3 test13f writes the
    // producer AFTER the consumer).
    for r in rules.iter_mut().chain(backward_rules.iter_mut()) {
        r.premise = order_premise(&r.premise);
    }
    let mut facts = FactIndex::from_iter(facts0);
    let mut bw = BwCtx::new(&backward_rules);
    bw.base = base;
    bw.resolver = resolver;
    if let Some(v) = visited {
        bw.visited = v;
    }
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
    // (correct; the fixpoint still terminates because conclusions are deduped). Pure-builtin
    // rules (no join atom) fire only in round 0.
    let rule_meta: Vec<(Vec<usize>, bool)> = rules
        .iter()
        .map(|r| {
            let joins: Vec<usize> =
                r.premise.iter().enumerate().filter(|(_, p)| is_join_atom(p)).map(|(i, _)| i).collect();
            // Non-monotonic premise operators — scoped negation/containment
            // AND the collectAllIn/forAllIn aggregations (their solution sets
            // grow with the closure) — force full re-evaluation every round.
            let has_neg = r
                .premise
                .iter()
                .any(|p| scope_op(&p[1]).is_some() || collect_op(&p[1]).is_some());
            let needs_bw = joins.iter().any(|&k| match &r.premise[k][1] {
                Term::Iri(i) => bw_any_var_pred || bw_concl_preds.contains(i.as_str()),
                _ => !backward_rules.is_empty(),
            });
            (joins, has_neg || needs_bw)
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

    let mut delta: FxHashSet<[Term; 3]> = facts.all.clone(); // round 0: every fact is "new"
    let mut first_round = true;
    loop {
        let mut produced: Vec<DerivationStep> = Vec::new();
        for (ri, rule) in rules.iter().enumerate() {
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
            let bindings: Vec<Binding> = if *needs_full || joins.is_empty() {
                // non-monotonic / backward-supported / constant rule: full evaluation
                // (negation + backward) every round, or round-0 only (constant).
                if *needs_full || first_round {
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
                        BW_DEPTH,
                    ));
                }
                bs
            };
            let (concl_blanks, concl_vars) = &concl_meta[ri];
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
                            .map(|l| {
                                (l.clone(), format!("{}{}_{}", sk_prefix, sk_counter, l))
                            })
                            .collect(),
                    )
                };
                for c in &rule.conclusion {
                    let c = match &sk {
                        Some(map) => rename_blanks(c, map),
                        None => c.clone(),
                    };
                    if let Some(g) = ground_triple(&c, &b) {
                        if !facts.contains(&g) {
                            // The supporting facts: premise patterns instantiated under b that
                            // are actual facts (excludes builtins / list structure).
                            let prem: Vec<[Term; 3]> = if mode == StepMode::Full {
                                rule.premise
                                    .iter()
                                    .filter_map(|p| ground_triple(p, &b))
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
                    st.gen_out.entry(g[0].clone()).or_default().push(g[2].clone());
                }
            }
        }
        first_round = false;
        if new_delta.is_empty() {
            break;
        }
        delta = new_delta;
    }
    (facts, steps)
}

/// Intern a term-level closure + derivation into the dictionary ([`reason_n3`] /
/// [`reason_n3_proof`] output form). Errors on formula-valued facts, which have
/// no dictionary representation (use [`reason_n3_terms`] for those documents).
fn intern_closure(
    dict: &mut Dict,
    facts: &FactIndex,
    steps: &[DerivationStep],
) -> Result<(Vec<[Id; 3]>, Vec<ProofStep>), String> {
    // First-class list values have no dictionary representation — expand them
    // into rdf:first/rest blank-node chains (one chain per list VALUE, shared
    // across the facts that mention it).
    let fact_rows: Vec<[Term; 3]> = facts.all.iter().cloned().collect();
    // The same expander also expands the proof rows below, so its cell labels must be
    // fresh against THOSE blanks too — seed it from both.
    let step_rows = steps.iter().flat_map(|(g, _, prem)| std::iter::once(g).chain(prem));
    let mut exp = ListExpander::new(fact_rows.iter().chain(step_rows));
    let (fact_rows, mut extra) = exp.expand_rows(&fact_rows);
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
        proof.push(ProofStep { conclusion, rule: *ri, premises });
    }
    Ok((out, proof))
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
    match_premise_seeded(premise, facts, &Binding::new(), None, bw, BW_DEPTH)
}

/// Match `premise` starting from an existing partial binding `seed`. For SEMI-NAIVE
/// evaluation, `delta_at = Some((delta, k))` restricts the join atom at premise index `k` to
/// match only the `delta` set (the newly-derived facts) rather than the full `facts` — so the
/// driver, by running once per join-atom index, considers only bindings that involve ≥1 new
/// fact. `None` = naive (every join atom matches all facts); also used for the negation
/// sub-formula recursion.
fn match_premise_seeded(
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
            if let Some(nb) = unify(&premise[k], fact, seed) {
                seeds.push(nb);
            }
        }
        if seeds.is_empty() {
            return Vec::new();
        }
        let rest: Vec<[Term; 3]> =
            premise.iter().enumerate().filter(|&(i, _)| i != k).map(|(_, p)| p.clone()).collect();
        let mut out = Vec::new();
        for s in &seeds {
            out.extend(match_premise_seeded(&rest, facts, s, None, bw, depth));
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
                let matches: Vec<Binding> = match apply_deep(&pat[0], &b) {
                    Term::Formula(ts) => {
                        let scope: Vec<[Term; 3]> = if matches!(op, ScopeOp::Supports) {
                            formula_closure(&ts, bw)
                        } else {
                            ts
                        };
                        formula_containment(&scope, inner, &b)
                    }
                    // `{}` parses as the literal true — the EMPTY formula:
                    // it includes nothing (and notIncludes everything).
                    Term::Lit(v, _, _) if v == "true" => formula_containment(&[], inner, &b),
                    _ => match_premise_seeded(inner, facts, &b, None, bw, depth),
                };
                if is_not {
                    if matches.is_empty() {
                        next.push(b);
                    }
                } else {
                    next.extend(matches);
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
        // predicates fully present before the stratum starts
        // ([`reason_n3_stratified`] is the stratified driver). A malformed
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
                    Some(match apply_deep(&pat[2], seed) {
                        Term::Formula(scope) => formula_containment(&scope, atoms, seed),
                        Term::Lit(v, _, _) if v == "true" => formula_containment(&[], atoms, seed),
                        _ => match_premise_seeded(atoms, facts, seed, None, bw, depth),
                    })
                };
                let subj = apply(&pat[0], &b);
                let Term::List(members) = &subj else { continue };
                match op {
                    CollectOp::CollectAll => {
                        let [template, clause, list_pat] = &members[..] else { continue };
                        let Some(sols) = solve(clause, &b) else { continue };
                        let collected =
                            Term::List(sols.iter().map(|s| apply_deep(template, s)).collect());
                        let mut nb = b.clone();
                        if unify_term(list_pat, &collected, &mut nb) {
                            next.push(nb);
                        }
                    }
                    CollectOp::ForAll => {
                        let [ca, cb] = &members[..] else { continue };
                        let Some(sols_a) = solve(ca, &b) else { continue };
                        if sols_a.iter().all(|s| matches!(solve(cb, s), Some(ss) if !ss.is_empty()))
                        {
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
                let head = apply(list_pos, b);
                // A first-class `( … )` list value, hand-written rdf:first/rest
                // rule structure, or a data list reached through a bound
                // variable (walked from the fact store).
                let members: Option<Vec<Term>> = match &head {
                    Term::List(ms) => Some(ms.clone()),
                    _ => fact_list(&head, facts),
                };
                if let Some(members) = members {
                    for (ix, m) in members.iter().enumerate() {
                        let mv = apply(m, b);
                        let target = match gen {
                            ListGen::Member | ListGen::In => mv,
                            // (?index ?value) pairs, 0-based (EYE list:iterate).
                            ListGen::Iterate => Term::List(vec![
                                Term::Lit(ix.to_string(), parser::XSD_INTEGER.into(), None),
                                mv,
                            ]),
                        };
                        let mut nb = b.clone();
                        if unify_term(var_pos, &target, &mut nb) {
                            next.push(nb);
                        }
                    }
                }
            }
            bindings = next;
        } else if let Some(f) = functional_builtin(&pat[1]) {
            bindings = bindings
                .into_iter()
                .filter_map(|b| eval_functional(f, &pat[0], &pat[2], facts, bw, b))
                .collect();
        } else if let Some(op) = binder_builtin(&pat[1]) {
            bindings = bindings.into_iter().filter_map(|b| eval_binder(op, &pat[0], &pat[2], b)).collect();
        } else if let Some(op) = builtin(&pat[1]) {
            bindings.retain(|b| eval_builtin(op, &pat[0], &pat[2], b));
        } else {
            // Join atom: selective FactIndex lookup (no full scan) for each current binding,
            // PLUS goal-directed resolution against the backward (`<=`) rules.
            let mut next = Vec::new();
            for b in &bindings {
                let (s_a, p_a, o_a) = (apply(&pat[0], b), apply(&pat[1], b), apply(&pat[2], b));
                // Virtual rdf:first / rdf:rest over a first-class list value —
                // cwm/EYE expose list structure to matching even though the
                // list is a term, not triples (`?L rdf:first ?X` computes).
                if let (Term::List(ms), Term::Iri(pi)) = (&s_a, &p_a) {
                    if pi == parser::RDF_FIRST || pi == parser::RDF_REST {
                        if !ms.is_empty() {
                            let val = if pi == parser::RDF_FIRST {
                                ms[0].clone()
                            } else {
                                Term::List(ms[1..].to_vec())
                            };
                            let mut nb = b.clone();
                            if unify_term(&pat[2], &val, &mut nb) {
                                next.push(nb);
                            }
                        }
                        continue; // a list term is never the subject of stored first/rest triples
                    }
                }
                let cands = facts.candidates(&s_a, &p_a, &o_a);
                for fact in &cands {
                    if let Some(nb) = unify(pat, fact, b) {
                        next.push(nb);
                    }
                }
                if !bw.rules.is_empty() && depth > 0 {
                    next.extend(backward_prove(pat, b, facts, bw, depth - 1));
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

/// Whether a premise pattern is a JOIN atom (matched against facts), as opposed to a builtin,
/// list generator/structure, or scoped-negation atom.
fn is_join_atom(pat: &[Term; 3]) -> bool {
    // A literal-list subject under rdf:first/rest is the VIRTUAL list-access
    // computation, not a store join.
    let virtual_list = matches!(&pat[0], Term::List(_))
        && matches!(&pat[1], Term::Iri(i) if i == parser::RDF_FIRST || i == parser::RDF_REST);
    builtin(&pat[1]).is_none()
        && functional_builtin(&pat[1]).is_none()
        && binder_builtin(&pat[1]).is_none()
        && list_generator(&pat[1]).is_none()
        && scope_op(&pat[1]).is_none()
        && collect_op(&pat[1]).is_none()
        && !virtual_list
}

/// Goal-directed (`<=`) resolution of one premise atom: for each backward rule whose
/// conclusion unifies with the goal (under the current binding `b`), prove that rule's
/// premise — recursively allowing joins, builtins, and further backward rules — and project
/// each proof back onto the goal's variables. SLD resolution with standardized-apart rule
/// variables and a depth bound (`depth` counts REMAINING backward applications).
fn backward_prove(pat: &[Term; 3], b: &Binding, facts: &FactIndex, bw: &BwCtx, depth: usize) -> Vec<Binding> {
    let goal = [apply(&pat[0], b), apply(&pat[1], b), apply(&pat[2], b)];
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
                        let val = apply(&walk(g, &sol), &sol);
                        if (val.is_ground() || matches!(val, Term::Formula(_)))
                            && !unify_term(g, &val, &mut nb)
                        {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
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
        (Term::Var(v), _) => {
            s.insert(v.clone(), cw.clone());
            true
        }
        (_, Term::Var(v)) => {
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
fn fact_list(head: &Term, facts: &FactIndex) -> Option<Vec<Term>> {
    let first = Term::Iri(parser::RDF_FIRST.into());
    let rest = Term::Iri(parser::RDF_REST.into());
    let nil = Term::Iri(parser::RDF_NIL.into());
    let empty = Term::List(Vec::new());
    let mut out = Vec::new();
    let mut cur = head.clone();
    let mut guard = 0;
    loop {
        if cur == nil || cur == empty {
            return Some(out);
        }
        if guard > 100_000 {
            return None;
        }
        guard += 1;
        let f = facts.ps.get(&(first.clone(), cur.clone()))?.first()?.clone();
        out.push(f);
        cur = facts.ps.get(&(rest.clone(), cur.clone()))?.first()?.clone();
    }
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

/// Try to unify pattern triple `pat` with ground fact `f`, extending binding `b`.
fn unify(pat: &[Term; 3], f: &[Term; 3], b: &Binding) -> Option<Binding> {
    let mut nb = b.clone();
    for i in 0..3 {
        if !unify_term(&pat[i], &f[i], &mut nb) {
            return None;
        }
    }
    Some(nb)
}

fn unify_term(pat: &Term, val: &Term, b: &mut Binding) -> bool {
    match (pat, val) {
        (Term::Var(v), _) => match b.get(v) {
            Some(existing) => existing == val,
            None => {
                b.insert(v.clone(), val.clone());
                true
            }
        },
        // First-class lists unify STRUCTURALLY: same length, members pairwise
        // (so `(?x)` matches `(17)` binding ?x=17 — cwm/EYE list unification).
        (Term::List(ps), Term::List(vs)) => {
            ps.len() == vs.len() && ps.iter().zip(vs).all(|(p, v)| unify_term(p, v, b))
        }
        // Unification THROUGH quoting: a `{ … }` pattern matches a `{ … }`
        // value when their triple multisets correspond under the binding
        // (pattern variables may bind quoted terms — cwm unify1/unify2).
        (Term::Formula(ps), Term::Formula(vs)) => formula_unify(ps, vs, b),
        // Quoted triples unify STRUCTURALLY, component by component (so
        // `<< ?s :p ?o >>` matches `<< :a :p :b >>` binding ?s/?o —
        // SPARQL-star / RDF-star triple-pattern semantics). [FABLE-5]
        (Term::Triple(ps), Term::Triple(vs)) => {
            ps.iter().zip(vs.iter()).all(|(p, v)| unify_term(p, v, b))
        }
        (other, _) => other == val,
    }
}

/// Multiset unification of two formula bodies under `b` (small formulae;
/// backtracking with a binding clone per branch).
fn formula_unify(ps: &[[Term; 3]], vs: &[[Term; 3]], b: &mut Binding) -> bool {
    if ps.len() != vs.len() {
        return false;
    }
    fn go(
        i: usize,
        ps: &[[Term; 3]],
        vs: &[[Term; 3]],
        used: &mut [bool],
        b: &Binding,
    ) -> Option<Binding> {
        if i == ps.len() {
            return Some(b.clone());
        }
        for j in 0..vs.len() {
            if used[j] {
                continue;
            }
            let mut nb = b.clone();
            if (0..3).all(|k| unify_term(&ps[i][k], &vs[j][k], &mut nb)) {
                used[j] = true;
                let done = go(i + 1, ps, vs, used, &nb);
                used[j] = false;
                if done.is_some() {
                    return done;
                }
            }
        }
        None
    }
    let mut used = vec![false; vs.len()];
    match go(0, ps, vs, &mut used, b) {
        Some(nb) => {
            *b = nb;
            true
        }
        None => false,
    }
}

/// Substitute bound variables in `t` (recursing into list members); returns the
/// term (possibly still containing free vars).
fn apply(t: &Term, b: &Binding) -> Term {
    match t {
        Term::Var(v) => b.get(v).cloned().unwrap_or_else(|| t.clone()),
        Term::List(ms) => Term::List(ms.iter().map(|m| apply(m, b)).collect()),
        Term::Triple(tr) => {
            Term::Triple(Box::new([apply(&tr[0], b), apply(&tr[1], b), apply(&tr[2], b)]))
        }
        _ => t.clone(),
    }
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
fn ground_triple(t: &[Term; 3], b: &Binding) -> Option<[Term; 3]> {
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
    let g = [apply_deep(&t[0], b), apply_deep(&t[1], b), apply_deep(&t[2], b)];
    if g.iter().all(instantiated) {
        Some(g)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
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

fn eval_builtin(op: Builtin, s: &Term, o: &Term, b: &Binding) -> bool {
    let (s, o) = (apply(s, b), apply(o, b));
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
                Builtin::StrMatches => regex::Regex::new(y).map(|re| re.is_match(x)).unwrap_or(false),
                Builtin::StrNotMatches => regex::Regex::new(y).map(|re| !re.is_match(x)).unwrap_or(false),
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
            let (Some(x), Some(y)) = (num(&s), num(&o)) else { return false };
            match op {
                Builtin::Gt => x > y,
                Builtin::Lt => x < y,
                Builtin::NotGt => x <= y,
                Builtin::NotLt => x >= y,
                Builtin::MathEq => x == y,
                Builtin::MathNe => x != y,
                _ => unreachable!(),
            }
        }
    }
}

/// Bidirectional binary builtins over a SINGLE subject term (not a `( … )` list): either
/// side may be the unknown, and evaluating binds it.
#[derive(Clone, Copy)]
enum Bidi {
    LogUri, // log:uri — IRI ↔ its text as an xsd:string (either direction)
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
fn eval_binder(op: Bidi, s: &Term, o: &Term, b: Binding) -> Option<Binding> {
    let (sv, ov) = (apply(s, &b), apply(o, &b));
    let mut nb = b;
    match op {
        Bidi::LogUri => match (&sv, &ov) {
            // forward: IRI subject → its text as a string literal
            (Term::Iri(i), _) => {
                let lit = Term::Lit(i.clone(), "http://www.w3.org/2001/XMLSchema#string".into(), None);
                if unify_term(o, &lit, &mut nb) {
                    Some(nb)
                } else {
                    None
                }
            }
            // reverse: string object → the IRI it names
            (Term::Var(_), Term::Lit(v, _, _)) => {
                if unify_term(s, &Term::Iri(v.clone()), &mut nb) {
                    Some(nb)
                } else {
                    None
                }
            }
            _ => None,
        },
    }
}

#[derive(Clone, Copy)]
enum ListGen {
    Member,  // ?list list:member ?x
    In,      // ?x list:in ?list
    Iterate, // ?list list:iterate (?index ?value) — 0-based, one binding per member
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

/// The formula-scope operators.
#[derive(Clone, Copy)]
enum ScopeOp {
    Includes,
    NotIncludes,
    Supports, // includes after closing the scope under its own rules
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

/// The scoped AGGREGATION / universal-quantification operators (EYE and the
/// N3 builtins spec; they share the scope convention of the [`ScopeOp`]s).
#[derive(Clone, Copy)]
enum CollectOp {
    /// `( ?template { clause } ?list ) log:collectAllIn ?scope` — findall.
    CollectAll,
    /// `( { clause-a } { clause-b } ) log:forAllIn ?scope` — every solution
    /// of clause-a extends to one of clause-b.
    ForAll,
}

fn collect_op(p: &Term) -> Option<CollectOp> {
    let Term::Iri(i) = p else { return None };
    match i.strip_prefix(LOG) {
        Some("collectAllIn") => Some(CollectOp::CollectAll),
        Some("forAllIn") => Some(CollectOp::ForAll),
        _ => None,
    }
}

/// Substitute bound variables in `t`, recursing into lists AND quoted
/// formulae (used where a formula value must be fully instantiated:
/// includes scopes, conclusion emission).
fn apply_deep(t: &Term, b: &Binding) -> Term {
    match t {
        Term::Var(v) => match b.get(v) {
            Some(val) => val.clone(),
            None => t.clone(),
        },
        Term::List(ms) => Term::List(ms.iter().map(|m| apply_deep(m, b)).collect()),
        Term::Formula(ts) => Term::Formula(
            ts.iter()
                .map(|r| [apply_deep(&r[0], b), apply_deep(&r[1], b), apply_deep(&r[2], b)])
                .collect(),
        ),
        Term::Triple(tr) => Term::Triple(Box::new([
            apply_deep(&tr[0], b),
            apply_deep(&tr[1], b),
            apply_deep(&tr[2], b),
        ])),
        _ => t.clone(),
    }
}

/// SYNTACTIC containment of `pattern` in `scope` (log:includes): every
/// pattern triple must match a scope triple (or be virtual list structure),
/// with pattern blanks as wildcards, pattern variables binding, and scope
/// terms — including its quantified variables — as opaque constants. Returns
/// one binding per complete match.
fn formula_containment(scope: &[[Term; 3]], pattern: &[[Term; 3]], seed: &Binding) -> Vec<Binding> {
    // Pattern existentials (blanks) become wildcard variables.
    let pat: Vec<[Term; 3]> = pattern
        .iter()
        .map(|r| {
            let w = |t: &Term| match t {
                Term::Blank(l) => Term::Var(format!("__w_{l}")),
                other => other.clone(),
            };
            [w(&r[0]), w(&r[1]), w(&r[2])]
        })
        .collect();
    let mut out = Vec::new();
    let mut budget = 100_000usize;
    containment_search(&pat, scope, seed.clone(), pat.len(), &mut out, &mut budget);
    out
}

fn containment_search(
    remaining: &[[Term; 3]],
    scope: &[[Term; 3]],
    b: Binding,
    defers_left: usize,
    out: &mut Vec<Binding>,
    budget: &mut usize,
) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    let Some((pat, rest)) = remaining.split_first() else {
        out.push(b);
        return;
    };
    // Virtual rdf:first/rest over a list value — list structure is part of
    // the formula's content for matching purposes (cwm builtins.n3 test2/4).
    if let Term::Iri(pi) = &pat[1] {
        if pi == parser::RDF_FIRST || pi == parser::RDF_REST {
            match apply(&pat[0], &b) {
                Term::List(ms) => {
                    if !ms.is_empty() {
                        let val = if pi == parser::RDF_FIRST {
                            ms[0].clone()
                        } else {
                            Term::List(ms[1..].to_vec())
                        };
                        let mut nb = b.clone();
                        if unify_term(&pat[2], &val, &mut nb) {
                            containment_search(rest, scope, nb, rest.len(), out, budget);
                        }
                    }
                    return;
                }
                Term::Var(_) if !rest.is_empty() && defers_left > 0 => {
                    // Subject not yet bound — try the other triples first.
                    let mut rotated: Vec<[Term; 3]> = rest.to_vec();
                    rotated.push(pat.clone());
                    containment_search(&rotated, scope, b, defers_left - 1, out, budget);
                    return;
                }
                _ => {} // fall through to plain scope matching
            }
        }
    }
    for st in scope {
        let mut nb = b.clone();
        if (0..3).all(|k| unify_term(&pat[k], &st[k], &mut nb)) {
            containment_search(rest, scope, nb, rest.len(), out, budget);
        }
    }
}

/// A parsed document's statements with rules re-encoded into their surface
/// triple form (log:semantics / log:parsedAsN3 result formulae).
fn reencode_statements(parsed: parser::Parsed) -> Vec<[Term; 3]> {
    let quote = |ts: &[[Term; 3]]| -> Term {
        if ts.is_empty() {
            // the empty formula IS the literal true
            Term::Lit("true".into(), parser::XSD_BOOLEAN.into(), None)
        } else {
            Term::Formula(ts.to_vec())
        }
    };
    let mut ts = parsed.facts;
    for r in &parsed.rules {
        ts.push([quote(&r.premise), Term::Iri(parser::LOG_IMPLIES.into()), quote(&r.conclusion)]);
    }
    for r in &parsed.backward_rules {
        ts.push([
            quote(&r.conclusion),
            Term::Iri(parser::LOG_IMPLIED_BY.into()),
            quote(&r.premise),
        ]);
    }
    ts
}

/// The forward closure of a quoted formula under its own `=>` rules
/// (log:supports / log:conclusion): the original triples plus everything a
/// fixpoint run over them derives.
fn formula_closure(ts: &[[Term; 3]], bw: &BwCtx) -> Vec<[Term; 3]> {
    // Import-cycle guard ([`VisitedDocs`]). If this exact formula's closure is already in
    // progress up the stack, re-closing it would recurse forever (the A→A / A→B→A / diamond
    // re-import non-termination path through a LIVE `log:semantics` resolver). Break the cycle
    // by returning the statements UNCLOSED — their own facts are preserved, no derivation is
    // lost that a finite closure would have added, and reasoning terminates. (No resolver, no
    // cycle: with `bw.resolver` `None` the set stays empty and this is a no-op.)
    let key = formula_key(ts);
    if !bw.visited.borrow_mut().insert(key) {
        return ts.to_vec();
    }

    let mut facts: Vec<[Term; 3]> = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    let mut backward: Vec<Rule> = Vec::new();
    for row in ts {
        match (&row[0], &row[1], &row[2]) {
            (Term::Formula(p), Term::Iri(i), Term::Formula(c)) if i == parser::LOG_IMPLIES => {
                rules.push(Rule { premise: p.clone(), conclusion: c.clone() });
            }
            (Term::Formula(c), Term::Iri(i), Term::Formula(p)) if i == parser::LOG_IMPLIED_BY => {
                backward.push(Rule { premise: p.clone(), conclusion: c.clone() });
            }
            _ => facts.push(row.clone()),
        }
    }
    let parsed =
        parser::Parsed { facts, rules, backward_rules: backward, base: bw.base.clone() };
    // Inherit the parent's import-cycle guard ([`VisitedDocs`]) so a `log:semantics` /
    // `log:content` document active up the stack is still recognised when its own closure
    // re-imports it through this nested run.
    let (closed, _steps) =
        run_closure(parsed, bw.resolver, Some(bw.visited.clone()), StepMode::None);
    // Original statements (including the rule statements, which cwm keeps in
    // log:conclusion output) plus the derivations.
    let mut seen: FxHashSet<[Term; 3]> = ts.iter().cloned().collect();
    let mut result: Vec<[Term; 3]> = ts.to_vec();
    for f in closed.all {
        if seen.insert(f.clone()) {
            result.push(f);
        }
    }
    bw.visited.borrow_mut().remove(&key);
    result
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
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The numeric value of a literal term (for `math:` builtins).
fn num(t: &Term) -> Option<f64> {
    match t {
        Term::Lit(v, _, _) => match v.as_str() {
            "INF" | "+INF" => Some(f64::INFINITY),
            "-INF" => Some(f64::NEG_INFINITY),
            "NaN" => Some(f64::NAN),
            _ => v.parse::<f64>().ok(),
        },
        _ => None,
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
        NumVal::Int(i) => Some(NumVal::Int(i.checked_neg()?)),
        NumVal::Dec(m, s) => Some(NumVal::Dec(m.checked_neg()?, s)),
        NumVal::F64(x) => Some(NumVal::F64(-x)),
    }
}

/// EYE `math:absoluteValue`, tier-preserving (`i128::checked_abs`, so `i128::MIN`
/// overflows → `None`; symmetric to [`numval_negate`]). [OPUS-4.8] sq-pbz04.5.1
#[inline]
fn numval_abs(v: NumVal) -> Option<NumVal> {
    match v {
        NumVal::Int(i) => Some(NumVal::Int(i.checked_abs()?)),
        NumVal::Dec(m, s) => Some(NumVal::Dec(m.checked_abs()?, s)),
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
        return v.parse::<f64>().ok().map(NumVal::F64);
    }
    if let Some((int, frac)) = v.split_once('.') {
        let digits = format!("{int}{frac}");
        if let Ok(m) = digits.parse::<i128>() {
            return Some(NumVal::Dec(m, frac.len() as u32));
        }
        return v.parse::<f64>().ok().map(NumVal::F64);
    }
    v.parse::<i128>()
        .ok()
        .map(NumVal::Int)
        .or_else(|| v.parse::<f64>().ok().map(NumVal::F64))
}

impl NumVal {
    fn to_f64(self) -> f64 {
        match self {
            NumVal::Int(i) => i as f64,
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
            m.checked_mul(10i128.checked_pow(s - from)?)
        };
        Some((up(ma, sa)?, up(mb, sb)?, s))
    }
    /// Value equality. The exact tiers delegate to the SHARED substrate
    /// [`sparq_substrate::numeric::Dec::cmp`] (the same scale-alignment the private
    /// tower's `aligned` did — `Dec::cmp` returns `None` on an alignment overflow,
    /// which falls back to the `f64` image exactly as before); any `f64` operand or
    /// an alignment overflow compares by `f64`. Byte-identical to the pre-adoption
    /// `aligned`-then-`==` path. [OPUS-4.8] sq-pbz04.5.1
    fn eq(a: NumVal, b: NumVal) -> bool {
        match (numval_to_subdec(a), numval_to_subdec(b)) {
            (Some(x), Some(y)) => match x.cmp(y) {
                Some(ord) => ord == std::cmp::Ordering::Equal,
                None => a.to_f64() == b.to_f64(), // scale-alignment overflow → f64 image
            },
            _ => a.to_f64() == b.to_f64(),
        }
    }
}

/// Drop trailing zero scale: Dec(700, 2) → Dec(7, 1); Dec(30, 1) → Dec(3, 0)
/// stays a Dec (decimal-typed inputs keep the decimal type, lexical `x.0`).
fn dec_norm(m: i128, s: u32) -> (i128, u32) {
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

/// Functional `math:`/`string:`/`list:`/`time:` builtins: the subject is a `( … )` list (or a
/// single value for the unary ops), and the object is computed.
#[derive(Clone, Copy)]
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

/// Evaluate a functional builtin `(members) op object`: resolve the list members under `b`,
/// compute, then either bind the object variable to the result or filter if it is ground.
fn eval_functional(
    f: Func,
    subj: &Term,
    obj: &Term,
    facts: &FactIndex,
    bw: &BwCtx,
    b: Binding,
) -> Option<Binding> {
    const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
    // log:dtlit needs the UNAPPLIED member terms: its reverse mode binds them by
    // decomposing a ground object literal into ( "lexical" datatype-IRI ).
    if let Func::Dtlit = f {
        let Term::List(members) = subj else { return None };
        if members.len() != 2 {
            return None;
        }
        let (m0, m1) = (apply(&members[0], &b), apply(&members[1], &b));
        let mut nb = b;
        return match (&m0, &m1) {
            // forward: ( "lex" xsd:dt ) → "lex"^^xsd:dt
            (Term::Lit(v, _, _), Term::Iri(dt)) => {
                let lit = Term::Lit(v.clone(), dt.clone(), None);
                unify_term(obj, &lit, &mut nb).then_some(nb)
            }
            // reverse: decompose a ground object literal into its parts
            _ => {
                let Term::Lit(v, dt, _) = apply(obj, &nb) else { return None };
                (unify_term(&members[0], &Term::Lit(v.clone(), XSD_STRING.into(), None), &mut nb)
                    && unify_term(&members[1], &Term::Iri(dt.clone()), &mut nb))
                .then_some(nb)
            }
        };
    }
    // math:negation is bidirectional in EYE: `?x math:negation 3` solves
    // ?x = -3 (the one reverse mode the suites rely on).
    if matches!(f, Func::Negation) && !subj.is_ground() && !matches!(subj, Term::List(_)) {
        let s_applied = apply(subj, &b);
        if !s_applied.is_ground() {
            let o_applied = apply(obj, &b);
            if let Some(v) = numval(&o_applied) {
                let negated = numval_negate(v)?;
                let mut nb = b;
                return unify_term(subj, &numval_term(negated), &mut nb).then_some(nb);
            }
            return None;
        }
    }
    // Reverse (object-bound) modes of the invertible unary builtins:
    // `?y math:sin 0` solves y = asin 0 (cwm trig.n3 test4), and
    // `?t time:inSeconds N` formats the epoch back to a UTC dateTime.
    if !subj.is_ground() && !matches!(subj, Term::List(_)) && !apply(subj, &b).is_ground() {
        let o_applied = apply(obj, &b);
        if o_applied.is_ground() {
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
                let secs = num(&o_applied)? as i64;
                let mut nb = b;
                let lit = Term::Lit(format_epoch(secs), XSD_STRING.into(), None);
                return unify_term(subj, &lit, &mut nb).then_some(nb);
            }
            if let Some(x) = numval(&o_applied).map(NumVal::to_f64) {
                if let Some(v) = inverse(x) {
                    if v.is_nan() {
                        return None;
                    }
                    let mut nb = b;
                    return unify_term(subj, &double_term(v), &mut nb).then_some(nb);
                }
            }
            return None;
        }
    }
    // Arguments: the list members (rule-local structure, a data list walked
    // from the fact store, or `rdf:nil` = the empty list), else a singleton
    // for the unary (math:/string:/time:) ops.
    let subj_applied = apply(subj, &b);
    let resolved_list: Option<Vec<Term>> = match &subj_applied {
        // First-class list value (already substituted by `apply`).
        Term::List(ms) => Some(ms.clone()),
        // A data list written as rdf:first/rest triples, via a bound variable.
        _ => fact_list(&subj_applied, facts).map(|ms| ms.iter().map(|m| apply(m, &b)).collect()),
    };
    let was_list = resolved_list.is_some();
    // The list:-namespace ops are only defined ON lists.
    if matches!(f, Func::Length | Func::First | Func::Last | Func::Append) && !was_list {
        return None;
    }
    let args: Vec<Term> = match resolved_list {
        Some(members) => members,
        None => vec![subj_applied.clone()],
    };
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
                number_term(distinct.len() as f64)
            }
            _ if was_list => number_term(args.len() as f64),
            _ => return None,
        },
        Func::Format => {
            // ( fmt args… ) string:format ?out — honest %s/%d/%f/%% subset; any other
            // directive (or an argument-count mismatch, which EYE throws on) fails.
            let fmt = lex(&args[0])?.to_string();
            let mut out = String::new();
            let mut argi = 1;
            let mut chars = fmt.chars();
            while let Some(c) = chars.next() {
                if c != '%' {
                    out.push(c);
                    continue;
                }
                match chars.next()? {
                    '%' => out.push('%'),
                    's' => {
                        out.push_str(lex(args.get(argi)?)?);
                        argi += 1;
                    }
                    'd' => {
                        let n = num(args.get(argi)?)?;
                        out.push_str(&(n as i64).to_string());
                        argi += 1;
                    }
                    'f' => {
                        let n = num(args.get(argi)?)?;
                        out.push_str(&format!("{n:.6}")); // C printf default precision
                        argi += 1;
                    }
                    _ => return None, // unsupported directive: fail, don't mangle
                }
            }
            if argi != args.len() {
                return None;
            }
            Term::Lit(out, XSD_STRING.into(), None)
        }
        Func::Scrape => {
            // ( str regex ) string:scrape ?out — first capture group of the first match.
            if args.len() != 2 {
                return None;
            }
            let re = regex::Regex::new(lex(&args[1])?).ok()?;
            let cap = re.captures(lex(&args[0])?)?.get(1)?.as_str().to_string();
            Term::Lit(cap, XSD_STRING.into(), None)
        }
        Func::Concat => {
            let mut s = String::new();
            for a in &args {
                match a {
                    // cwm coerces typed literals to their canonical VALUE
                    // string ("0"^^xsd:boolean → "false", 0E1 → "0").
                    Term::Lit(v, dt, _) => match dt.strip_prefix("http://www.w3.org/2001/XMLSchema#") {
                        Some("boolean") => s.push_str(if v == "0" || v == "false" { "false" } else { "true" }),
                        Some("integer" | "decimal" | "float" | "double") => {
                            match numval(a) {
                                Some(NumVal::Int(i)) => s.push_str(&i.to_string()),
                                Some(NumVal::Dec(m, sc)) => {
                                    let (m, sc) = dec_norm(m, sc);
                                    if sc == 0 {
                                        s.push_str(&m.to_string());
                                    } else {
                                        let Term::Lit(lex, _, _) = numval_term(NumVal::Dec(m, sc)) else { return None };
                                        s.push_str(&lex);
                                    }
                                }
                                Some(NumVal::F64(f)) => {
                                    if f.fract() == 0.0 && f.abs() < 9.007e15 {
                                        s.push_str(&(f as i64).to_string());
                                    } else {
                                        s.push_str(&format!("{f}"));
                                    }
                                }
                                None => s.push_str(v),
                            }
                        }
                        _ => s.push_str(v),
                    },
                    // cwm coerces IRI arguments to their text (concatenation.n3 s01).
                    Term::Iri(i) => s.push_str(i),
                    _ => return None,
                }
            }
            Term::Lit(s, "http://www.w3.org/2001/XMLSchema#string".into(), None)
        }
        Func::Length => number_term(args.len() as f64),
        Func::StrLength => number_term(lex(&args[0])?.chars().count() as f64),
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
            let keep_extra: &[u8] = if matches!(f, Func::EncodeForUriCwm) {
                b"#'()~"
            } else {
                b"/"
            };
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
        Func::LogConclusion => match &args[..] {
            [Term::Formula(ts)] => Term::Formula(formula_closure(ts, bw)),
            _ => return None,
        },
        Func::Semantics | Func::Content => match &args[..] {
            [Term::Iri(doc)] => {
                let text = bw.resolver.and_then(|r| r(doc))?;
                if matches!(f, Func::Content) {
                    Term::Lit(text, XSD_STRING.into(), None)
                } else {
                    // cwm-faithful: `log:semantics` returns the document's PARSED statements
                    // (rules re-encoded as `=>` triples), NOT their closure — the closure is
                    // taken later by `log:supports` / `log:conclusion`, which is where the
                    // import-cycle guard ([`VisitedDocs`]) applies. Resolution itself does no
                    // recursion, so no marking is needed here.
                    let parsed = parser::parse_with_base(&text, doc).ok()?;
                    Term::Formula(reencode_statements(parsed))
                }
            }
            _ => return None,
        },
        Func::ParsedAsN3 => match &args[..] {
            [Term::Lit(src, _, _)] => {
                let parsed = parser::parse_with_base(src, &bw.base).ok()?;
                Term::Formula(reencode_statements(parsed))
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
                    other => merged.extend(fact_list(other, facts)?),
                }
            }
            Term::List(merged)
        }
        Func::Replace => {
            // ( str pattern replacement ) string:replace ?out — regex replace-all.
            if args.len() != 3 {
                return None;
            }
            let re = regex::Regex::new(lex(&args[1])?).ok()?;
            let out = re.replace_all(lex(&args[0])?, lex(&args[2])?).into_owned();
            Term::Lit(out, "http://www.w3.org/2001/XMLSchema#string".into(), None)
        }
        Func::Year | Func::Month | Func::Day | Func::Hours | Func::Minutes | Func::Seconds
        | Func::DayOfWeek | Func::InSeconds => {
            number_term(datetime_part(lex(&args[0])?, f)? as f64)
        }
        Func::TimeZone => {
            // Only an EXPLICIT numeric offset is a time zone (cwm: `Z`/absent
            // yield nothing).
            let s = lex(&args[0])?;
            let t = s.split('T').nth(1)?;
            let off = t.find(['+', '-']).map(|i| &t[i..])?;
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
            if let Some(exact) = eval_exact(f, &args) {
                exact
            } else {
                let nvals: Vec<NumVal> = args.iter().map(numval).collect::<Option<_>>()?;
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
                if trig_family || any_double {
                    double_term(v)
                } else {
                    number_term(v)
                }
            }
        }
    };
    // Bind the object variable; if it is already GROUND, compare NUMERICALLY
    // when both sides are numbers (so `15.5` matches a computed `15.5e0` and
    // exact-decimal results match their reference lexical forms), else
    // structurally.
    let mut nb = b;
    let obj_applied = apply(obj, &nb);
    if obj_applied.is_ground() {
        if let (Some(x), Some(y)) = (numval(&obj_applied), numval(&result)) {
            return NumVal::eq(x, y).then_some(nb);
        }
    }
    if unify_term(obj, &result, &mut nb) {
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
fn eval_exact(f: Func, args: &[Term]) -> Option<Term> {
    let vals: Vec<NumVal> = args.iter().map(numval).collect::<Option<_>>()?;
    if vals.iter().any(|v| matches!(v, NumVal::F64(_))) {
        return None;
    }
    let any_dec = vals.iter().any(|v| matches!(v, NumVal::Dec(_, _)));
    let pair = || -> Option<(i128, i128, u32)> {
        if vals.len() == 2 {
            NumVal::aligned(vals[0], vals[1])
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
    let unary_int = |round: fn(i128, i128) -> i128| -> Option<NumVal> {
        let (m, s) = match vals[0] {
            NumVal::Int(i) => (i, 0u32),
            NumVal::Dec(m, s) => (m, s),
            NumVal::F64(_) => return None,
        };
        let pow = 10i128.checked_pow(s)?;
        let v = round(m, pow);
        Some(if s == 0 { NumVal::Int(v) } else { NumVal::Dec(v, 0) })
    };
    // The exact add / subtract / multiply DELEGATE to the shared substrate
    // `Dec` (byte-identical `(mant, scale)`: `+`/`-` keep the max scale, `*` sums
    // the scales — the SAME i128 mantissa ops the private tower did). `renorm`
    // keeps EYE's result-type rule (all-integer in → integer out; any decimal in
    // → decimal out). [OPUS-4.8] sq-pbz04.5.1
    use sparq_substrate::numeric::Dec as SubDec;
    let out = match f {
        Func::Sum => {
            let mut acc = SubDec { mant: 0, scale: 0 };
            for &v in &vals {
                acc = acc.checked_add(numval_to_subdec(v)?)?;
            }
            renorm(acc.mant, acc.scale)
        }
        Func::Product => {
            let mut acc = SubDec { mant: 1, scale: 0 };
            for &v in &vals {
                acc = acc.checked_mul(numval_to_subdec(v)?)?;
            }
            renorm(acc.mant, acc.scale)
        }
        Func::Difference => {
            if vals.len() != 2 {
                return None;
            }
            let d = numval_to_subdec(vals[0])?.checked_sub(numval_to_subdec(vals[1])?)?;
            renorm(d.mant, d.scale)
        }
        Func::Max | Func::Min => {
            // Compare via the shared substrate `Dec::cmp` (the same scale-aligned
            // i128 order as the private tower's `aligned`-then-`>`); an alignment
            // overflow (`cmp` → `None`) falls through to the f64 path, matching the
            // pre-adoption `aligned(..)?` behaviour. The WINNING ORIGINAL operand is
            // returned unchanged (its own scale preserved). [OPUS-4.8] sq-pbz04.5.1
            let mut best = vals[0];
            for &v in &vals[1..] {
                let ord = numval_to_subdec(best)?.cmp(numval_to_subdec(v)?)?;
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
                return None;
            }
            let mut scale = 0u32;
            while a % b != 0 && scale < 34 {
                a = a.checked_mul(10)?;
                scale += 1;
            }
            if a % b != 0 {
                return None; // not exact — f64 fallback
            }
            if any_dec || scale > 0 {
                NumVal::Dec(a / b, scale)
            } else {
                NumVal::Int(a / b)
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
                    let r = a.checked_rem(b)?;
                    NumVal::Int(if r != 0 && (r < 0) != (b < 0) { r.checked_add(b)? } else { r })
                }
                _ => return None,
            }
        }
        Func::IntegerQuotient => {
            let (a, b, _) = pair()?;
            if b == 0 {
                return None;
            }
            NumVal::Int(a.div_euclid(b))
        }
        // Tier-preserving unary sign ops via the shared adapter helpers (i128
        // `checked_neg`/`checked_abs` on the substrate `Dec` mantissa — `None` on
        // `i128::MIN` overflow, exactly as before). `vals[0]` is never `F64` here
        // (the leading guard returns early on any `F64`), so the helper's `F64` arm
        // is unreachable in this path. [OPUS-4.8] sq-pbz04.5.1
        Func::Negation => match vals[0] {
            NumVal::F64(_) => return None,
            v => numval_negate(v)?,
        },
        Func::AbsoluteValue => match vals[0] {
            NumVal::F64(_) => return None,
            v => numval_abs(v)?,
        },
        // round-half-UP: floor(x + 1/2) — what the suite references encode
        // (-2.5 → -2, 0.5 → 1, 2.5 → 3). rounded keeps the decimal TYPE
        // (`-3.0`), while floor/ceiling return integers — both per the cwm
        // reference outputs.
        Func::Rounded => unary_int(|m, pow| (m + pow / 2).div_euclid(pow))?,
        Func::Floor => match unary_int(|m, pow| m.div_euclid(pow))? {
            NumVal::Dec(m, _) => NumVal::Int(m),
            v => v,
        },
        Func::Ceiling => match unary_int(|m, pow| -((-m).div_euclid(pow)))? {
            NumVal::Dec(m, _) => NumVal::Int(m),
            v => v,
        },
        Func::Exponentiation => {
            // base^exp exactly for an integer exponent ≥ 0 (cwm: 2.7² = 7.29).
            if vals.len() != 2 {
                return None;
            }
            let (NumVal::Int(e), base) = (vals[1], vals[0]) else { return None };
            if !(0..=64).contains(&e) {
                return None;
            }
            let (m, sc) = match base {
                NumVal::Int(i) => (i, 0u32),
                NumVal::Dec(m, sc) => (m, sc),
                NumVal::F64(_) => return None,
            };
            let mut acc: i128 = 1;
            for _ in 0..e {
                acc = acc.checked_mul(m)?;
            }
            let scale = sc.checked_mul(e as u32)?;
            if scale > 34 {
                return None;
            }
            if any_dec {
                NumVal::Dec(acc, scale)
            } else {
                NumVal::Int(acc)
            }
        }
        _ => return None, // trig/log: f64 path
    };
    Some(numval_term(out))
}

/// Extract a component of an `xsd:dateTime`/`xsd:date` lexical form for `time:` builtins.
/// Lexical: `[-]YYYY-MM-DD[Thh:mm:ss[.sss]][Z|±hh:mm]`.
fn datetime_part(s: &str, f: Func) -> Option<i64> {
    let (date, time) = s.split_once('T').unwrap_or((s, ""));
    let neg = date.starts_with('-');
    let mut dparts = date.trim_start_matches('-').split('-');
    match f {
        Func::Year => dparts.next()?.parse::<i64>().ok().map(|y| if neg { -y } else { y }),
        Func::Month => dparts.nth(1)?.parse().ok(),
        Func::Day => dparts.nth(2)?.parse().ok(),
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
            part.split('.').next().unwrap_or(part).parse().ok()
        }
        Func::DayOfWeek | Func::InSeconds => {
            // Missing components default (cwm: "2002" = 2002-01-01T00:00:00).
            let mut dp = date.trim_start_matches('-').split('-');
            let y: i64 = {
                let y = dp.next()?.parse::<i64>().ok()?;
                if neg { -y } else { y }
            };
            let m: i64 = dp.next().and_then(|x| x.parse().ok()).unwrap_or(1);
            let d: i64 = dp.next().and_then(|x| x.parse().ok()).unwrap_or(1);
            let days = days_from_civil(y, m, d);
            if matches!(f, Func::DayOfWeek) {
                return Some((days + 4).rem_euclid(7)); // 1970-01-01 = Thursday
            }
            let t = time.split(['+', 'Z']).next().unwrap_or(time);
            let t = match t.rfind('-') {
                Some(i) => &t[..i], // a '-' inside the TIME part starts a tz offset
                None => t,
            };
            let mut tp = t.split(':');
            let hh: i64 = tp.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            let mi: i64 = tp.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            let ss: i64 = tp
                .next()
                .and_then(|x| x.split('.').next().unwrap_or(x).parse().ok())
                .unwrap_or(0);
            // Explicit ±hh:mm offset shifts back to UTC; Z/absent = UTC.
            let mut offset = 0i64;
            if let Some(tpart) = s.split_once('T').map(|(_, t)| t) {
                if let Some(i) = tpart.find(['+', '-']) {
                    let sign = if tpart.as_bytes()[i] == b'-' { -1 } else { 1 };
                    let mut op = tpart[i + 1..].split(':');
                    let oh: i64 = op.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                    let om: i64 = op.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                    offset = sign * (oh * 3600 + om * 60);
                }
            }
            Some(days * 86400 + hh * 3600 + mi * 60 + ss - offset)
        }
        _ => None,
    }
}

/// Days since 1970-01-01 of the civil date y-m-d (Howard Hinnant's algorithm).
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
    format!(
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
fn number_term(v: f64) -> Term {
    if v.fract() == 0.0 && v.abs() < 9.007e15 {
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
                        "quoted-triple subject {other:?} is not an IRI or blank node (RDF 1.2 triple terms admit no other subject kind)"
                    ))
                }
            };
            let Term::Iri(p) = &tr[1] else {
                return Err(format!(
                    "quoted-triple predicate {:?} is not an IRI (RDF 1.2 triple terms admit no other predicate kind)",
                    tr[1]
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
            return Err(format!("term {t:?} inside a quoted triple has no dictionary representation"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // BW_DEPTH.
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
            eval_exact(f, args),
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
            eval_exact(Func::Sum, &[lit("0.1", XSD_DEC), lit("0.2", XSD_DEC)]),
            Some(lit("0.3", XSD_DEC))
        );
        // ('2.7' '2') math:difference = 0.7 EXACTLY (f64 gives 0.7000000000000002).
        assert_eq!(
            eval_exact(Func::Difference, &[lit("2.7", XSD_DEC), plain("2")]),
            Some(lit("0.7", XSD_DEC))
        );
        // 2.7 * 2.7 = 7.29 exactly (scale 1 * scale 1 → scale 2).
        assert_eq!(
            eval_exact(Func::Product, &[lit("2.7", XSD_DEC), lit("2.7", XSD_DEC)]),
            Some(lit("7.29", XSD_DEC))
        );
        // A trailing-zero scale is normalised by numval_term/dec_norm: 1 + 0.20 → "1.2".
        assert_eq!(
            eval_exact(Func::Sum, &[plain("1"), lit("0.20", XSD_DEC)]),
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
            eval_exact(Func::Sum, &[plain(&a.to_string()), plain(&b.to_string())]),
            Some(lit(&sum.to_string(), XSD_INT)),
            "sum of a > i64::MAX integer stays an exact xsd:integer via the substrate Dec carrier"
        );
        // Product of two large integers, still exact within i128.
        let big = 3_037_000_500i128; // ~sqrt(i128::MAX)/... well within i128 when squared? no — pick safe
        let sq = big.checked_mul(big).unwrap();
        assert_eq!(
            eval_exact(Func::Product, &[plain(&big.to_string()), plain(&big.to_string())]),
            Some(lit(&sq.to_string(), XSD_INT))
        );
        // Difference crossing the i64 boundary.
        let hi = (i64::MAX as i128) + 500;
        assert_eq!(
            eval_exact(Func::Difference, &[plain(&hi.to_string()), plain("500")]),
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
            assert_eq!(eval_exact(Func::Sum, &args), None, "{} has no exact tier", special);
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
        assert_eq!(eval_exact(Func::Quotient, &[plain("6"), plain("2")]), Some(lit("3", XSD_INT)));
        // exact terminating quotient → decimal.
        assert_eq!(eval_exact(Func::Quotient, &[plain("1"), plain("4")]), Some(lit("0.25", XSD_DEC)));
        // non-terminating → None (f64 fallback), NOT a rounded decimal.
        assert_eq!(eval_exact(Func::Quotient, &[plain("1"), plain("3")]), None);
        // remainder: divisor-sign (Python %): -2 mod 4 = 2, 2 mod -4 = -2.
        assert_eq!(eval_exact(Func::Remainder, &[plain("-2"), plain("4")]), Some(lit("2", XSD_INT)));
        assert_eq!(eval_exact(Func::Remainder, &[plain("2"), plain("-4")]), Some(lit("-2", XSD_INT)));
        // integerQuotient: floor division.
        assert_eq!(eval_exact(Func::IntegerQuotient, &[plain("-7"), plain("2")]), Some(lit("-4", XSD_INT)));
        // floor/ceiling collapse a decimal to an xsd:integer; rounded keeps the "N.0" decimal.
        assert_eq!(eval_exact(Func::Floor, &[lit("2.7", XSD_DEC)]), Some(lit("2", XSD_INT)));
        assert_eq!(eval_exact(Func::Ceiling, &[lit("2.1", XSD_DEC)]), Some(lit("3", XSD_INT)));
        assert_eq!(eval_exact(Func::Rounded, &[lit("2.5", XSD_DEC)]), Some(lit("3.0", XSD_DEC)));
    }

    #[test]
    fn diff_negation_abs_tier_preserving() {
        // Negation / abs preserve the tier and scale (Dec stays Dec with its scale).
        assert_eq!(eval_exact(Func::Negation, &[plain("5")]), Some(lit("-5", XSD_INT)));
        assert_eq!(eval_exact(Func::Negation, &[lit("3.50", XSD_DEC)]), Some(lit("-3.5", XSD_DEC)));
        assert_eq!(eval_exact(Func::AbsoluteValue, &[lit("-3.50", XSD_DEC)]), Some(lit("3.5", XSD_DEC)));
        // A > i64 integer negates exactly (i128 checked_neg on the substrate Dec carrier).
        let big = (i64::MAX as i128) + 7;
        assert_eq!(
            eval_exact(Func::Negation, &[plain(&big.to_string())]),
            Some(lit(&(-big).to_string(), XSD_INT))
        );
    }
}
