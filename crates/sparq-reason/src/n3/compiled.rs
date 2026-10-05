//! [FABLE-5] sq-zgbso.3 (epic sq-zgbso, issue #1582) — **id-level compiled-rule
//! evaluation** for the scoped N3 subset the Solid access-control rules use, behind the
//! opt-in `compiled-rules` feature.
//!
//! The text engine ([`crate::reason_n3`]) runs its semi-naive fixpoint at the **string
//! term level**: every `materialize_*`-class caller serializes its id-level facts to N3
//! text, re-parses them, joins on `String` terms, and interns the closure back to ids —
//! per call (3× for the stratified ACP pipeline). This module removes that round-trip:
//!
//! 1. **Compile once** ([`compile`]): parse rule text with the existing [`super::parser`]
//!    (the same parse, premise ordering and atom classification the text engine uses)
//!    and lower each rule to an IR whose constants live in a **symbol table** of ground
//!    terms — the maintainer's "URIs pre-indexed with ids attached" representation.
//! 2. **Bind per dictionary** ([`CompiledRuleSet::bind`]): intern the symbol table into
//!    the caller's [`Dict`], so every rule constant is a `u32` id in the SAME id space as
//!    the caller's facts.
//! 3. **Evaluate at the id level** ([`BoundRuleSet::eval`] / the [`eval`] convenience):
//!    a semi-naive forward fixpoint over `[Id; 3]` facts whose join atoms drive the
//!    SHARED [`sparq_substrate::join`] kernels ([`sparq_substrate::join::build_table`] +
//!    [`sparq_substrate::join::hash_probe_serial`], the same monomorphic bodies the
//!    SPARQL engine and the RDFS materialiser drive — deliberately **not** a third join
//!    implementation; this module only reshapes each combined row back into its
//!    variable-slot layout, the thin layout-adapter pattern of
//!    `crate::substrate_join`). Builtins evaluate per surviving row, converting ids to
//!    terms only at the builtin boundary and interning any minted term back into the
//!    caller's `Dict` — facts never pass through text.
//!
//! # Honest scope — a SUBSET of N3, not the full engine
//!
//! Exactly the constructs the sparq-solid access-control corpus
//! (`rules/{common,wac,acp-a,acp-b,acp-c}.n3` + the sq-zgbso.1 ODRL spike rules) uses:
//!
//! * plain triple-pattern join atoms (variables allowed in any position, including
//!   predicate);
//! * **RDF 1.2 / RDF-star quoted triples** (`<< s p o >>` / `<<( s p o )>>`) in any
//!   PREMISE position and as a ground constant anywhere. A ground quoted triple is an
//!   ordinary symbol-table constant — [`Dict`] content-addresses a triple term by its
//!   component ids, so it binds to the SAME id a store-loaded `<<( s p o )>>` has. A
//!   quoted triple that still contains variables compiles to a component-indexed
//!   `UnpackTriple` step: the enclosing join binds the candidate's term id, then the
//!   unpack step reads its three component ids straight out of the dictionary record
//!   (no term reconstruction, no allocation) and binds/filters each — the id-level
//!   analogue of the text engine's structural quoted-triple unification. Nesting is
//!   admitted through the OBJECT, which is the only nesting a dictionary triple term
//!   has: RDF 1.2 gives a triple term an IRI-or-blank subject and an IRI predicate, so a
//!   pattern quoting a nested triple in SUBJECT position (which the term-level engine
//!   does match) is a loud compile error here rather than a rule that never fires;
//! * **store-scoped `log:notIncludes`** — negation as failure against the current fact
//!   store, with the engine's no-retraction semantics. Set-equivalence with
//!   [`crate::reason_n3`] therefore requires every negated predicate to be
//!   **stratum-complete** (fully derived before the stratum that negates it runs) —
//!   the same §3.5 stratification discipline the WAC/ACP pipeline already obeys;
//! * `log:uri` (both directions), `log:equalTo` / `log:notEqualTo`;
//! * `string:concatenation` (with the engine's typed-literal value coercion),
//!   `string:encodeForUri`, `string:scrape` (constant regex), `string:notGreaterThan`
//!   (LEXICAL comparison, exactly like the text engine).
//!
//! Everything else is a **loud [`compile`] error**, never a silent divergence: backward
//! (`<=`) rules, conclusion existentials (blank nodes), `log:includes`/`log:supports`,
//! list builtins/generators, `math:`/`time:` builtins, the remaining `string:` builtins,
//! formula- or list-valued facts, and rules whose builtin inputs no premise can bind.
//! Two quoted-triple shapes are deliberately out too, each for a stated reason:
//!
//! * a quoted triple with variables inside a **`log:notIncludes` body** — the anti-join
//!   runs a flat list of plain patterns, with no place to interleave an unpack step;
//! * a quoted triple with variables in a **conclusion** — minting a triple term from
//!   bound components can violate RDF 1.2's structural constraints (a component may
//!   bind to a literal subject or a non-IRI predicate) at derivation time, and
//!   [`BoundRuleSet::eval`] has no channel to report that failure, so it would have to
//!   choose between a panic and a silent drop. The text engine derives those heads.
//!
//! Full-N3 conformance stays with the text engine.
//!
//! The result-equivalence oracle is `tests/compiled_equivalence.rs`: on the WAC/ACP/
//! ODRL-spike corpus the compiled closure equals the [`crate::reason_n3`] closure **as a
//! set** over the same rules + facts.

use super::model::Term;
use super::parser;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{is_inline, Dict, Id, TermParts};
use sparq_substrate::join::{self as sjoin, JoinKeys, NoBudget};
use sparq_substrate::rows::{Row, NO_ID};

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// A term slot in a compiled atom: a pre-interned constant (index into the rule set's
/// symbol table) or a rule-local variable slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CTerm {
    /// Index into [`CompiledRuleSet::symbols`] (resolved to an [`Id`] by `bind`).
    Const(u32),
    /// Rule-local variable slot (a column of the binding [`Row`]).
    Var(u32),
}

/// One triple pattern, pre-analysed at compile time into the exact layout the shared
/// hash-join kernels need: equi-join key column pairs for the already-bound variables,
/// write-back slots for the new ones, and constant filters per position.
#[derive(Clone, Debug)]
struct PatternStep {
    /// `(binding column = var slot, candidate column 0..3)` for every position holding a
    /// variable that is already bound when this step runs — the [`JoinKeys::key_cols`].
    key_cols: Vec<(usize, usize)>,
    /// `(var slot, candidate column)` for every position holding a NOT-yet-bound
    /// variable. A repeated new variable within one atom appears twice; the second
    /// occurrence becomes an equality check when the combined row is reshaped.
    new_writes: Vec<(usize, usize)>,
    /// Per-position constant (symbol index) — a candidate pre-filter.
    consts: [Option<u32>; 3],
}

/// One component position of a [`Step::UnpackTriple`]: bind the component id into a
/// slot the quotation is the FIRST premise occurrence of, or match it against an
/// already-bound variable / a constant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TripleSlot {
    Bind(u32),
    Check(CTerm),
}

/// One ordered premise step of a compiled rule.
#[derive(Clone, Debug)]
enum Step {
    /// A join atom over the fact store (semi-naive delta restriction applies here).
    Pattern(PatternStep),
    /// Store-scoped `log:notIncludes`: drop every binding row for which the inner
    /// pattern has at least one match in the CURRENT store (no retraction — see the
    /// module docs for the stratum-completeness requirement). `wildcards` are the
    /// inner pattern's free-variable slots, reset after the check so match bindings
    /// never leak outward (engine parity: a `notIncludes` match binds nothing).
    NotIncludes {
        pats: Vec<PatternStep>,
        wildcards: Vec<usize>,
    },
    /// Component-indexed match through an RDF 1.2 quoted triple that still contains
    /// variables. The enclosing [`Step::Pattern`] (or an outer `UnpackTriple`) writes
    /// the candidate term's id into `src`; this step keeps the row only if that id is
    /// really a triple term and each of its three component ids binds or matches per
    /// `comps`. Components apply LEFT TO RIGHT, so `<< ?x :p ?x >>` binds on the
    /// subject and filters on the object — the text engine's unification order.
    /// Nested quotation emits a further `UnpackTriple` over a component slot.
    UnpackTriple { src: u32, comps: [TripleSlot; 3] },
    /// `log:equalTo` / `log:notEqualTo` — exact term (id) comparison.
    IdCompare { a: CTerm, b: CTerm, negate: bool },
    /// `string:notGreaterThan` — LEXICAL `a <= b` over two literals' lexical forms.
    StrNotGreaterThan { a: CTerm, b: CTerm },
    /// `log:uri`, forward: an IRI subject's text as an `xsd:string` object.
    UriToText {
        iri: CTerm,
        out: CTerm,
        out_bound: bool,
    },
    /// `log:uri`, reverse: a literal object's lexical form as the subject IRI.
    TextToUri {
        text: CTerm,
        out: CTerm,
        out_bound: bool,
    },
    /// `string:encodeForUri` — RFC 3986 percent-encoding ([`super::encode_for_uri`]).
    EncodeForUri {
        arg: CTerm,
        out: CTerm,
        out_bound: bool,
    },
    /// `string:concatenation` over a `( … )` subject list, with the text engine's
    /// typed-literal value coercion.
    Concat {
        args: Vec<CTerm>,
        out: CTerm,
        out_bound: bool,
    },
    /// `string:scrape` — `( str regex )`: the first capture group of the first match.
    /// The regex is a compile-time constant, pre-compiled into
    /// [`CompiledRuleSet::regexes`] (`None` = invalid pattern ⇒ the step fails every
    /// row, exactly like the text engine's per-evaluation `Regex::new(..).ok()?`).
    Scrape {
        arg: CTerm,
        regex: usize,
        out: CTerm,
        out_bound: bool,
    },
}

/// One compiled forward rule.
#[derive(Clone, Debug)]
struct CompiledRule {
    /// Ordered premise steps (the engine's own `order_premise` ordering).
    steps: Vec<Step>,
    /// Conclusion triples (every variable provably bound by the premise).
    conclusion: Vec<[CTerm; 3]>,
    /// Indices of the [`Step::Pattern`] steps — the semi-naive delta positions.
    join_steps: Vec<usize>,
    /// Whether the rule carries scoped negation: non-monotonic, so it re-evaluates
    /// against the FULL store every round (the engine's `needs_full` discipline).
    needs_full: bool,
    /// Binding-row width: variable slots incl. per-negation wildcard slots.
    n_slots: usize,
}

/// A compiled, dictionary-independent N3 rule set: the [`compile`] output.
///
/// Constants (IRIs, literals, blank labels) live in a symbol table of ground terms;
/// [`CompiledRuleSet::bind`] interns that table into a concrete [`Dict`] so evaluation
/// runs entirely on `u32` ids. The set also carries the rule document's own ground
/// facts (e.g. the `acp-c.n3` mode-mapping triples), which [`BoundRuleSet::eval`] adds
/// to the closure exactly as [`crate::reason_n3`] does.
#[derive(Debug)]
pub struct CompiledRuleSet {
    symbols: Vec<Term>,
    regexes: Vec<Option<regex::Regex>>,
    facts: Vec<[u32; 3]>,
    rules: Vec<CompiledRule>,
}

impl CompiledRuleSet {
    /// Number of compiled forward rules.
    pub fn n_rules(&self) -> usize {
        self.rules.len()
    }

    /// Number of ground facts carried by the rule document itself.
    pub fn n_facts(&self) -> usize {
        self.facts.len()
    }

    /// Intern the rule vocabulary (the symbol table) into `dict`, producing a rule set
    /// whose constants are ids in `dict`'s id space.
    ///
    /// The returned [`BoundRuleSet`] is only meaningful against the SAME `dict` (ids are
    /// dictionary-specific); pass that dictionary — and facts interned in it — to
    /// [`BoundRuleSet::eval`].
    pub fn bind(&self, dict: &mut Dict) -> BoundRuleSet<'_> {
        let syms = self
            .symbols
            .iter()
            .map(|t| intern_ground(dict, t))
            .collect();
        BoundRuleSet {
            compiled: self,
            syms,
        }
    }
}

/// A [`CompiledRuleSet`] bound to one dictionary: every rule constant resolved to its
/// [`Id`]. Produced by [`CompiledRuleSet::bind`].
pub struct BoundRuleSet<'a> {
    compiled: &'a CompiledRuleSet,
    syms: Vec<Id>,
}

/// Parse N3 rule text and lower it to an id-level [`CompiledRuleSet`].
///
/// Uses the existing [`super::parser`] front end and the engine's own premise ordering,
/// so a compiled rule evaluates its builtins at exactly the positions the text engine
/// would. Any construct outside the module's documented subset is a loud `Err` naming
/// the offending construct — see the module docs for the honest envelope.
///
/// # Errors
///
/// Returns `Err` on a parse failure or on any unsupported construct (backward rules,
/// unsupported builtins, conclusion existentials, unresolvable builtin inputs,
/// formula-/list-valued facts, unbound conclusion variables).
///
/// # Examples
///
/// ```
/// use sparq_reason::n3::compiled::compile;
/// let rules = compile("@prefix : <http://ex/> . { ?x a :Man } => { ?x a :Mortal } .")?;
/// assert_eq!(rules.n_rules(), 1);
/// # Ok::<(), String>(())
/// ```
pub fn compile(src: &str) -> Result<CompiledRuleSet, String> {
    let parsed = parser::parse(src)?;
    if !parsed.backward_rules.is_empty() {
        return Err("compiled-rules: backward (`<=`) rules are not in the compiled subset (goal-directed resolution stays with the text engine)".into());
    }
    let mut c = Compiler::default();
    for f in &parsed.facts {
        c.lower_fact(f)?;
    }
    for r in &parsed.rules {
        c.lower_rule(r)?;
    }
    Ok(CompiledRuleSet {
        symbols: c.symbols,
        regexes: c.regexes,
        facts: c.facts,
        rules: c.rules,
    })
}

/// Parse an N3 **fact** document (no rules) and intern its ground triples into `dict` —
/// the text-side loader for tests/harnesses. Production callers of the compiled path
/// should feed `[Id; 3]` facts straight from their own store instead; this helper exists
/// so a differential against [`crate::reason_n3`] can start both paths from one text.
///
/// # Errors
///
/// Returns `Err` on a parse failure, if the document contains rules, or if a fact term
/// has no dictionary representation (quoted formulae, first-class lists, variables).
///
/// # Examples
///
/// ```
/// use sparq_reason::n3::compiled::intern_facts;
/// let mut dict = sparq_core::dict::Dict::new();
/// let facts = intern_facts(&mut dict, "<http://ex/s> <http://ex/p> <http://ex/o> .")?;
/// assert_eq!(facts.len(), 1);
/// # Ok::<(), String>(())
/// ```
pub fn intern_facts(dict: &mut Dict, src: &str) -> Result<Vec<[Id; 3]>, String> {
    let parsed = parser::parse(src)?;
    if !parsed.rules.is_empty() || !parsed.backward_rules.is_empty() {
        return Err("intern_facts: the document contains rules — compile() them instead".into());
    }
    let mut out = Vec::with_capacity(parsed.facts.len());
    for t in &parsed.facts {
        out.push([
            intern_ground_checked(dict, &t[0])?,
            intern_ground_checked(dict, &t[1])?,
            intern_ground_checked(dict, &t[2])?,
        ]);
    }
    Ok(out)
}

/// Bind `rules` to `dict` and evaluate the forward fixpoint over `facts` — the one-call
/// entry shape (`eval(dict, facts, compiled rules) -> Vec<[Id; 3]>`, no text round-trip).
///
/// Returns the full ground closure (input facts + the rule document's own facts + every
/// derivation) as a de-duplicated list; treat it as a SET (order is unspecified).
///
/// # Examples
///
/// ```
/// use sparq_reason::n3::compiled::{compile, eval, intern_facts};
/// let rules = compile("@prefix : <http://ex/> . { ?x a :Man } => { ?x a :Mortal } .")?;
/// let mut dict = sparq_core::dict::Dict::new();
/// let facts = intern_facts(&mut dict, "@prefix : <http://ex/> . :Socrates a :Man .")?;
/// let closure = eval(&mut dict, &facts, &rules);
/// assert_eq!(closure.len(), 2); // the fact + the derivation
/// # Ok::<(), String>(())
/// ```
pub fn eval(dict: &mut Dict, facts: &[[Id; 3]], rules: &CompiledRuleSet) -> Vec<[Id; 3]> {
    rules.bind(dict).eval(dict, facts)
}

// ---------------------------------------------------------------------------
// Compile front end
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Compiler {
    sym_map: FxHashMap<Term, u32>,
    symbols: Vec<Term>,
    regexes: Vec<Option<regex::Regex>>,
    facts: Vec<[u32; 3]>,
    rules: Vec<CompiledRule>,
}

/// Rule-local lowering state: variable-name → slot, plus which slots are bound so far.
#[derive(Default)]
struct RuleCtx {
    slots: FxHashMap<String, u32>,
    bound: FxHashSet<u32>,
    n_slots: usize,
}

impl RuleCtx {
    fn slot(&mut self, name: &str) -> u32 {
        if let Some(&s) = self.slots.get(name) {
            return s;
        }
        let s = self.n_slots as u32;
        self.slots.insert(name.to_string(), s);
        self.n_slots += 1;
        s
    }
    fn fresh(&mut self) -> u32 {
        let s = self.n_slots as u32;
        self.n_slots += 1;
        s
    }
}

impl Compiler {
    fn sym(&mut self, t: &Term) -> Result<u32, String> {
        match t {
            Term::Iri(_) | Term::Lit(..) | Term::Blank(_) => {}
            // A GROUND quoted triple is an ordinary constant: `Dict` content-addresses
            // an RDF 1.2 triple term by its component ids, so `bind` resolves it to the
            // same id a store-loaded `<<( s p o )>>` carries. Validate RDF 1.2's
            // structural constraints HERE, at compile time, so `intern_ground` can never
            // fail at bind time.
            Term::Triple(_) if t.is_ground() => {
                super::n3_term_to_oxrdf(t).map_err(|e| format!("compiled-rules: {e}"))?;
            }
            other => {
                return Err(format!(
                    "compiled-rules: term {other:?} is not a compilable constant (quoted formulae / lists / variables — and a quoted triple that still contains variables — are outside the compiled subset here; the text engine handles them)"
                ))
            }
        }
        if let Some(&ix) = self.sym_map.get(t) {
            return Ok(ix);
        }
        let ix = self.symbols.len() as u32;
        self.sym_map.insert(t.clone(), ix);
        self.symbols.push(t.clone());
        Ok(ix)
    }

    fn lower_fact(&mut self, t: &[Term; 3]) -> Result<(), String> {
        let f = [self.sym(&t[0])?, self.sym(&t[1])?, self.sym(&t[2])?];
        self.facts.push(f);
        Ok(())
    }

    /// A builtin argument: a constant symbol or an ALREADY-BOUND variable. Unbound
    /// inputs are a loud error — after the engine's own premise ordering, an input no
    /// preceding atom can bind means the rule could never fire through this builtin.
    fn input(&mut self, t: &Term, ctx: &RuleCtx, what: &str) -> Result<CTerm, String> {
        match t {
            Term::Var(v) => match ctx.slots.get(v) {
                Some(&s) if ctx.bound.contains(&s) => Ok(CTerm::Var(s)),
                _ => Err(format!(
                    "compiled-rules: {what} input ?{v} is not bound by any preceding pattern (the rule could never fire)"
                )),
            },
            other => Ok(CTerm::Const(self.sym(other)?)),
        }
    }

    /// A builtin OUTPUT position: a fresh variable binds the result; a bound variable
    /// or a constant turns the step into an equality filter.
    fn output(&mut self, t: &Term, ctx: &mut RuleCtx) -> Result<(CTerm, bool), String> {
        match t {
            Term::Var(v) => {
                let s = ctx.slot(v);
                if ctx.bound.contains(&s) {
                    Ok((CTerm::Var(s), true))
                } else {
                    ctx.bound.insert(s);
                    Ok((CTerm::Var(s), false))
                }
            }
            other => Ok((CTerm::Const(self.sym(other)?), true)),
        }
    }

    /// Lower one join atom. In a `log:notIncludes` body (`naf` = the outer bound set +
    /// this negation's wildcard rename map), variables that the OUTER premise has bound
    /// are correlated columns; everything else is a per-negation wildcard slot whose
    /// binding is discarded after the existence check.
    ///
    /// A quoted-triple position that still contains variables cannot be a candidate
    /// pre-filter, so it binds the candidate's TERM id into a fresh anonymous slot and
    /// defers the component match to a [`Step::UnpackTriple`] appended to `pending`
    /// (which the caller pushes AFTER this pattern's join step — the component ids only
    /// exist once the join has chosen a candidate).
    #[allow(clippy::type_complexity)]
    fn lower_pattern(
        &mut self,
        atom: &[Term; 3],
        ctx: &mut RuleCtx,
        naf: Option<(
            &FxHashSet<u32>,
            &mut FxHashMap<String, u32>,
            &mut Vec<usize>,
        )>,
        pending: &mut Vec<Step>,
    ) -> Result<PatternStep, String> {
        let mut key_cols = Vec::new();
        let mut new_writes = Vec::new();
        let mut consts = [None, None, None];
        let mut quoted: Vec<(u32, &[Term; 3])> = Vec::new();
        // Wildcard/local bound-tracking for a negation body lives in `ctx.bound` too:
        // the caller snapshots and restores it around the whole body.
        let mut naf = naf;
        for (i, t) in atom.iter().enumerate() {
            match t {
                Term::Var(v) => {
                    let slot = match &mut naf {
                        Some((outer_bound, rename, wildcards)) => {
                            let outer = ctx.slots.get(v).copied();
                            match outer {
                                Some(s) if outer_bound.contains(&s) => s,
                                _ => match rename.get(v) {
                                    Some(&w) => w,
                                    None => {
                                        let w = ctx.fresh();
                                        rename.insert(v.clone(), w);
                                        wildcards.push(w as usize);
                                        w
                                    }
                                },
                            }
                        }
                        None => ctx.slot(v),
                    };
                    if ctx.bound.contains(&slot) {
                        key_cols.push((slot as usize, i));
                    } else {
                        new_writes.push((slot as usize, i));
                    }
                }
                Term::Iri(_) | Term::Lit(..) | Term::Blank(_) => consts[i] = Some(self.sym(t)?),
                // A GROUND quoted triple is just a constant id — an ordinary candidate
                // pre-filter, no unpacking needed.
                Term::Triple(_) if t.is_ground() => consts[i] = Some(self.sym(t)?),
                Term::Triple(tr) => {
                    if naf.is_some() {
                        return Err(format!(
                            "compiled-rules: quoted triple {t:?} with variables inside a log:notIncludes body is not in the compiled subset (the anti-join runs a flat list of plain patterns, with no place for a component-unpack step)"
                        ));
                    }
                    // Bind the candidate's TERM id into a fresh anonymous slot; the
                    // deferred unpack step decomposes it after the join.
                    let dst = ctx.fresh();
                    new_writes.push((dst as usize, i));
                    quoted.push((dst, tr.as_ref()));
                }
                other => {
                    return Err(format!(
                        "compiled-rules: {other:?} in a triple pattern is not in the compiled subset (quoted formulae / lists are matched by the text engine only)"
                    ))
                }
            }
        }
        // Bind every new variable only AFTER the whole atom is analysed, so a variable
        // that also occurs in a plain position of this same atom binds from the join
        // (which runs first) and the unpack merely CHECKS it.
        for &(s, _) in &new_writes {
            ctx.bound.insert(s as u32);
        }
        for (dst, tr) in quoted {
            self.lower_quoted(tr, dst, ctx, pending)?;
        }
        Ok(PatternStep {
            key_cols,
            new_writes,
            consts,
        })
    }

    /// Lower a quoted-triple PATTERN that still contains variables. The term id lands in
    /// `src` (written by the enclosing pattern join, or by an outer unpack) and this
    /// emits the [`Step::UnpackTriple`] that decomposes it: a first-occurrence variable
    /// binds, an already-bound variable or a constant filters, and a nested quotation
    /// takes a fresh slot plus its own (later) unpack step. Components are lowered
    /// left-to-right and evaluated in that same order.
    fn lower_quoted(
        &mut self,
        tr: &[Term; 3],
        src: u32,
        ctx: &mut RuleCtx,
        pending: &mut Vec<Step>,
    ) -> Result<(), String> {
        let mut comps = [TripleSlot::Bind(0); 3];
        let mut nested: Vec<(u32, &[Term; 3])> = Vec::new();
        for (i, t) in tr.iter().enumerate() {
            // RDF 1.2's structural constraints, checked STATICALLY — the same ones
            // `super::n3_term_to_oxrdf` enforces when interning a ground quotation. A
            // dictionary triple term's subject is an IRI or blank node and its predicate
            // an IRI, so a pattern quoting anything else (a literal, or — the one that
            // bites — a NESTED quotation in subject position, which the id-level
            // representation nests only through the OBJECT) could never match any id.
            // Loud here, rather than a rule that silently never fires.
            match (i, t) {
                (0, Term::Lit(..) | Term::Triple(_)) => {
                    return Err(format!(
                        "compiled-rules: quoted-triple subject {t:?} is not an IRI, a blank node or a variable (RDF 1.2 triple terms admit no other subject kind, so no dictionary triple term can match)"
                    ))
                }
                (1, Term::Lit(..) | Term::Blank(_) | Term::Triple(_)) => {
                    return Err(format!(
                        "compiled-rules: quoted-triple predicate {t:?} is not an IRI or a variable (RDF 1.2 triple terms admit no other predicate kind, so no dictionary triple term can match)"
                    ))
                }
                _ => {}
            }
            comps[i] = match t {
                Term::Var(v) => {
                    let s = ctx.slot(v);
                    if ctx.bound.contains(&s) {
                        TripleSlot::Check(CTerm::Var(s))
                    } else {
                        ctx.bound.insert(s);
                        TripleSlot::Bind(s)
                    }
                }
                Term::Triple(inner) if !t.is_ground() => {
                    let s = ctx.fresh();
                    ctx.bound.insert(s);
                    nested.push((s, inner.as_ref()));
                    TripleSlot::Bind(s)
                }
                // Ground terms (incl. a ground nested quotation) are constants; a
                // formula / list component is `sym`'s loud error.
                other => TripleSlot::Check(CTerm::Const(self.sym(other)?)),
            };
        }
        pending.push(Step::UnpackTriple { src, comps });
        for (s, inner) in nested {
            self.lower_quoted(inner, s, ctx, pending)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn lower_rule(&mut self, rule: &super::model::Rule) -> Result<(), String> {
        // The engine's OWN premise ordering (builtins after their producers), so the
        // compiled evaluation order matches the text engine's exactly.
        let premise = super::order_premise(&rule.premise);
        let mut ctx = RuleCtx::default();
        let mut steps: Vec<Step> = Vec::new();
        let mut join_steps: Vec<usize> = Vec::new();
        let mut needs_full = false;

        for atom in &premise {
            let p = &atom[1];
            // Classify in the SAME precedence order as the text engine's premise loop.
            if let Some(op) = super::scope_op(p) {
                if !matches!(op, super::ScopeOp::NotIncludes) {
                    return Err(
                        "compiled-rules: log:includes / log:supports are not in the compiled subset (only store-scoped log:notIncludes)".into(),
                    );
                }
                if matches!(atom[0], Term::Formula(_) | Term::List(_)) {
                    return Err(
                        "compiled-rules: a formula-scoped log:notIncludes subject is not in the compiled subset (formula values have no id representation)".into(),
                    );
                }
                let Term::Formula(inner) = &atom[2] else {
                    return Err(
                        "compiled-rules: the log:notIncludes object must be a quoted { … } formula"
                            .into(),
                    );
                };
                // Lower the body against a SNAPSHOT of the outer bound set: wildcards
                // become fresh slots bound only within the body, and the outer bound
                // set is restored afterwards (match bindings never leak — engine parity).
                let outer_bound = ctx.bound.clone();
                let mut rename: FxHashMap<String, u32> = FxHashMap::default();
                let mut wildcards: Vec<usize> = Vec::new();
                let mut pats = Vec::with_capacity(inner.len());
                // The anti-join runs plain patterns only; `lower_pattern` rejects the
                // one shape that would want a deferred step, so this stays empty.
                let mut naf_pending: Vec<Step> = Vec::new();
                for ia in inner {
                    if !super::is_join_atom(ia) {
                        return Err(
                            "compiled-rules: only plain triple patterns are supported inside log:notIncludes (no builtins / nested formulae)".into(),
                        );
                    }
                    pats.push(self.lower_pattern(
                        ia,
                        &mut ctx,
                        Some((&outer_bound, &mut rename, &mut wildcards)),
                        &mut naf_pending,
                    )?);
                }
                debug_assert!(
                    naf_pending.is_empty(),
                    "a log:notIncludes body must not produce deferred steps"
                );
                ctx.bound = outer_bound;
                steps.push(Step::NotIncludes { pats, wildcards });
                needs_full = true;
                continue;
            }
            if super::collect_op(p).is_some() {
                return Err(
                    "compiled-rules: log:collectAllIn / log:forAllIn are not in the compiled subset (scoped aggregation stays with the text engine)".into(),
                );
            }
            if super::list_generator(p).is_some() {
                return Err(format!(
                    "compiled-rules: list generator {p:?} is not in the compiled subset"
                ));
            }
            if let Some(f) = super::functional_builtin(p) {
                match f {
                    super::Func::Concat => {
                        let Term::List(members) = &atom[0] else {
                            return Err(
                                "compiled-rules: string:concatenation needs a ( … ) subject list"
                                    .into(),
                            );
                        };
                        let mut args = Vec::with_capacity(members.len());
                        for m in members {
                            args.push(self.input(m, &ctx, "string:concatenation")?);
                        }
                        let (out, out_bound) = self.output(&atom[2], &mut ctx)?;
                        steps.push(Step::Concat {
                            args,
                            out,
                            out_bound,
                        });
                    }
                    super::Func::Scrape => {
                        let Term::List(members) = &atom[0] else {
                            return Err(
                                "compiled-rules: string:scrape needs a ( str regex ) subject list"
                                    .into(),
                            );
                        };
                        if members.len() != 2 {
                            return Err(
                                "compiled-rules: string:scrape needs exactly ( str regex )".into(),
                            );
                        }
                        let arg = self.input(&members[0], &ctx, "string:scrape")?;
                        let Term::Lit(pat, ..) = &members[1] else {
                            return Err(
                                "compiled-rules: the string:scrape regex must be a literal constant in the compiled subset".into(),
                            );
                        };
                        let regex = self.regexes.len();
                        self.regexes.push(regex::Regex::new(pat).ok());
                        let (out, out_bound) = self.output(&atom[2], &mut ctx)?;
                        steps.push(Step::Scrape {
                            arg,
                            regex,
                            out,
                            out_bound,
                        });
                    }
                    super::Func::EncodeForUri => {
                        if matches!(atom[0], Term::List(_)) {
                            return Err(
                                "compiled-rules: string:encodeForUri takes a single value subject in the compiled subset".into(),
                            );
                        }
                        let arg = self.input(&atom[0], &ctx, "string:encodeForUri")?;
                        let (out, out_bound) = self.output(&atom[2], &mut ctx)?;
                        steps.push(Step::EncodeForUri {
                            arg,
                            out,
                            out_bound,
                        });
                    }
                    _ => {
                        return Err(format!(
                            "compiled-rules: functional builtin {p:?} is not in the compiled subset"
                        ))
                    }
                }
                continue;
            }
            if super::binder_builtin(p).is_some() {
                // log:uri — direction resolved at COMPILE time from which side is bound
                // (the engine resolves it at eval time; after order_premise the two
                // agree for every rule this subset admits).
                let subj_avail = match &atom[0] {
                    Term::Var(v) => ctx.slots.get(v).is_some_and(|s| ctx.bound.contains(s)),
                    Term::Iri(_) | Term::Lit(..) | Term::Blank(_) => true,
                    _ => false,
                };
                let obj_avail = match &atom[2] {
                    Term::Var(v) => ctx.slots.get(v).is_some_and(|s| ctx.bound.contains(s)),
                    Term::Iri(_) | Term::Lit(..) | Term::Blank(_) => true,
                    _ => false,
                };
                if subj_avail {
                    let iri = self.input(&atom[0], &ctx, "log:uri")?;
                    let (out, out_bound) = self.output(&atom[2], &mut ctx)?;
                    steps.push(Step::UriToText {
                        iri,
                        out,
                        out_bound,
                    });
                } else if obj_avail {
                    let text = self.input(&atom[2], &ctx, "log:uri")?;
                    let (out, out_bound) = self.output(&atom[0], &mut ctx)?;
                    steps.push(Step::TextToUri {
                        text,
                        out,
                        out_bound,
                    });
                } else {
                    return Err("compiled-rules: log:uri needs one bound side (no preceding pattern binds either)".into());
                }
                continue;
            }
            if let Some(op) = super::builtin(p) {
                let a = self.input(&atom[0], &ctx, "comparison builtin")?;
                let b = self.input(&atom[2], &ctx, "comparison builtin")?;
                match op {
                    super::Builtin::LogEq => steps.push(Step::IdCompare {
                        a,
                        b,
                        negate: false,
                    }),
                    super::Builtin::LogNe => steps.push(Step::IdCompare { a, b, negate: true }),
                    super::Builtin::StrNotGt => steps.push(Step::StrNotGreaterThan { a, b }),
                    _ => {
                        return Err(format!(
                            "compiled-rules: comparison builtin {p:?} is not in the compiled subset"
                        ))
                    }
                }
                continue;
            }
            // A plain join atom. Any quoted-triple position defers a component-unpack
            // step, which must run immediately AFTER this pattern's join (the semi-naive
            // delta position stays the `Step::Pattern` index recorded above).
            join_steps.push(steps.len());
            let mut pending: Vec<Step> = Vec::new();
            let ps = self.lower_pattern(atom, &mut ctx, None, &mut pending)?;
            steps.push(Step::Pattern(ps));
            steps.append(&mut pending);
        }

        // Conclusions: every variable must be bound by the (positive part of the)
        // premise; existential blanks would need the engine's per-firing skolemizer.
        let mut conclusion = Vec::with_capacity(rule.conclusion.len());
        for t in &rule.conclusion {
            let mut row = [CTerm::Const(0); 3];
            for (i, term) in t.iter().enumerate() {
                row[i] = match term {
                    Term::Var(v) => match ctx.slots.get(v) {
                        Some(&s) if ctx.bound.contains(&s) => CTerm::Var(s),
                        _ => {
                            return Err(format!(
                                "compiled-rules: conclusion variable ?{v} is not bound by the premise"
                            ))
                        }
                    },
                    Term::Blank(b) => {
                        return Err(format!(
                            "compiled-rules: existential blank _:{b} in a conclusion is not in the compiled subset (no per-firing skolemization)"
                        ))
                    }
                    Term::Triple(_) if !term.is_ground() => {
                        return Err(format!(
                            "compiled-rules: quoted triple {term:?} with variables in a conclusion is not in the compiled subset (minting a triple term from bound components can violate RDF 1.2's structural constraints at derivation time, which eval() has no channel to report — the text engine derives it)"
                        ))
                    }
                    other => CTerm::Const(self.sym(other)?),
                };
            }
            conclusion.push(row);
        }

        self.rules.push(CompiledRule {
            steps,
            conclusion,
            join_steps,
            needs_full,
            n_slots: ctx.n_slots,
        });
        Ok(())
    }
}

fn intern_ground(dict: &mut Dict, t: &Term) -> Id {
    match t {
        Term::Iri(i) => dict.intern_iri(i),
        Term::Lit(v, dt, lang) => dict.intern_lit(v, dt, lang.as_deref()),
        Term::Blank(b) => dict.intern_blank(b),
        // A ground quoted triple interns through `Dict`'s content-addressed RDF 1.2
        // triple-term path — the SAME id a store-loaded `<<( s p o )>>` gets. `sym` /
        // `intern_ground_checked` already rejected every shape the conversion cannot
        // represent, so it cannot fail here.
        Term::Triple(_) => match super::n3_term_to_oxrdf(t) {
            Ok(ox) => dict.intern(&ox),
            Err(e) => {
                unreachable!("compiled-rules symbol tables hold only representable ground terms: {e}")
            }
        },
        // compile()/intern_facts() admit only ground symbols.
        _ => unreachable!("compiled-rules symbol tables hold only ground terms"),
    }
}

fn intern_ground_checked(dict: &mut Dict, t: &Term) -> Result<Id, String> {
    match t {
        Term::Iri(_) | Term::Lit(..) | Term::Blank(_) => Ok(intern_ground(dict, t)),
        Term::Triple(_) if t.is_ground() => {
            let ox = super::n3_term_to_oxrdf(t).map_err(|e| format!("intern_facts: {e}"))?;
            Ok(dict.intern(&ox))
        }
        other => Err(format!(
            "intern_facts: term {other:?} has no dictionary representation"
        )),
    }
}

/// The component ids of an RDF 1.2 quoted-triple id, or `None` for any other term kind.
///
/// [`Dict`] stores a triple term STRUCTURALLY — as the ids of its three already-interned
/// components — so this is a record read, with no term reconstruction and no allocation:
/// exactly the id-level accessor a component-indexed quoted-triple match needs. Inline
/// integer ids and [`NO_ID`] carry no record, so they answer `None` rather than probing.
fn triple_components(dict: &Dict, id: Id) -> Option<[Id; 3]> {
    if id == NO_ID || is_inline(id) {
        return None;
    }
    match dict.term_parts(id) {
        TermParts::Triple(ids) => Some(ids),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Id-level evaluation
// ---------------------------------------------------------------------------

/// The id-level fact store: the closure set plus a predicate index (the analogue of the
/// text engine's `FactIndex`, over `u32` ids instead of `String` terms).
#[derive(Default)]
struct FactStore {
    all: FxHashSet<[Id; 3]>,
    by_pred: FxHashMap<Id, Vec<[Id; 3]>>,
    list: Vec<[Id; 3]>,
}

impl FactStore {
    fn insert(&mut self, t: [Id; 3]) -> bool {
        if !self.all.insert(t) {
            return false;
        }
        self.by_pred.entry(t[1]).or_default().push(t);
        self.list.push(t);
        true
    }
}

impl BoundRuleSet<'_> {
    /// Run the semi-naive forward fixpoint over `facts` (ids in the dictionary this set
    /// was [bound](CompiledRuleSet::bind) to) and return the full ground closure —
    /// input facts + the rule document's own facts + every derivation, de-duplicated.
    /// Treat the result as a SET (order is unspecified).
    ///
    /// `dict` is needed mutably because the string builtins (`log:uri`,
    /// `string:concatenation`, …) MINT terms (e.g. `urn:sparq:pair?…` principals) that
    /// must be interned into the caller's id space; join atoms never touch the
    /// dictionary.
    pub fn eval(&self, dict: &mut Dict, facts: &[[Id; 3]]) -> Vec<[Id; 3]> {
        let mut store = FactStore::default();
        for f in facts {
            store.insert(*f);
        }
        for f in &self.compiled.facts {
            store.insert([
                self.syms[f[0] as usize],
                self.syms[f[1] as usize],
                self.syms[f[2] as usize],
            ]);
        }
        // Round 0: every fact is "new". Rules with scoped negation re-evaluate fully
        // each round (needs_full); pure-constant rules fire in round 0 only — the text
        // engine's exact round discipline.
        let mut delta: Vec<[Id; 3]> = store.list.clone();
        let mut first_round = true;
        loop {
            let mut produced: Vec<[Id; 3]> = Vec::new();
            for rule in &self.compiled.rules {
                if rule.needs_full || rule.join_steps.is_empty() {
                    if rule.needs_full || first_round {
                        self.run_rule(rule, &store, None, dict, &mut produced);
                    }
                } else {
                    // Semi-naive: once per join position, with that pattern restricted
                    // to the delta (dedup happens at store insertion).
                    for &k in &rule.join_steps {
                        self.run_rule(rule, &store, Some((&delta, k)), dict, &mut produced);
                    }
                }
            }
            let mut new_delta: Vec<[Id; 3]> = Vec::new();
            for f in produced {
                if store.insert(f) {
                    new_delta.push(f);
                }
            }
            first_round = false;
            if new_delta.is_empty() {
                break;
            }
            delta = new_delta;
        }
        store.list
    }

    fn resolve(&self, t: CTerm, row: &Row) -> Id {
        match t {
            CTerm::Const(ix) => self.syms[ix as usize],
            CTerm::Var(v) => row[v as usize],
        }
    }

    /// Bind-or-filter a builtin's output position (compile-time static mode).
    fn unify_out(&self, row: &mut Row, out: CTerm, out_bound: bool, val: Id) -> bool {
        match out {
            CTerm::Const(ix) => self.syms[ix as usize] == val,
            CTerm::Var(v) => {
                if out_bound {
                    row[v as usize] == val
                } else {
                    row[v as usize] = val;
                    true
                }
            }
        }
    }

    /// Candidate facts for one pattern step: the delta (when this is the restricted
    /// position) or the store, narrowed by the predicate index and the constant
    /// pre-filters. Returned as substrate probe rows `[s, p, o]`.
    fn candidates(
        &self,
        p: &PatternStep,
        store: &FactStore,
        delta: Option<&[[Id; 3]]>,
    ) -> Vec<Row> {
        let pc = p.consts[1].map(|ix| self.syms[ix as usize]);
        let sc = p.consts[0].map(|ix| self.syms[ix as usize]);
        let oc = p.consts[2].map(|ix| self.syms[ix as usize]);
        let keep = |t: &[Id; 3]| {
            sc.is_none_or(|c| t[0] == c)
                && pc.is_none_or(|c| t[1] == c)
                && oc.is_none_or(|c| t[2] == c)
        };
        let source: &[[Id; 3]] = match (delta, pc) {
            (Some(d), _) => d,
            (None, Some(pid)) => store.by_pred.get(&pid).map_or(&[][..], |v| v.as_slice()),
            (None, None) => &store.list,
        };
        source
            .iter()
            .filter(|t| keep(t))
            .map(|t| Row::from_slice(t))
            .collect()
    }

    fn run_rule(
        &self,
        rule: &CompiledRule,
        store: &FactStore,
        delta_at: Option<(&[[Id; 3]], usize)>,
        dict: &mut Dict,
        out: &mut Vec<[Id; 3]>,
    ) {
        let empty: Row = std::iter::repeat_n(NO_ID, rule.n_slots).collect();
        let mut rows: Vec<Row> = vec![empty];
        for (si, step) in rule.steps.iter().enumerate() {
            match step {
                Step::Pattern(p) => {
                    let delta = delta_at.and_then(|(d, k)| (k == si).then_some(d));
                    let cands = self.candidates(p, store, delta);
                    rows = join_pattern(&rows, p, &cands, rule.n_slots);
                }
                Step::NotIncludes { pats, wildcards } => {
                    rows = self.anti_join(rows, pats, wildcards, store, rule.n_slots);
                }
                Step::UnpackTriple { src, comps } => {
                    let mut next = Vec::with_capacity(rows.len());
                    'unpack: for mut row in rows {
                        // Not a triple term (or unbound): the quotation cannot match.
                        let Some(ids) = triple_components(dict, row[*src as usize]) else {
                            continue;
                        };
                        for (c, id) in comps.iter().zip(ids) {
                            match c {
                                TripleSlot::Bind(s) => row[*s as usize] = id,
                                TripleSlot::Check(t) => {
                                    if self.resolve(*t, &row) != id {
                                        continue 'unpack;
                                    }
                                }
                            }
                        }
                        next.push(row);
                    }
                    rows = next;
                }
                Step::IdCompare { a, b, negate } => {
                    rows.retain(|r| {
                        let eq = self.resolve(*a, r) == self.resolve(*b, r);
                        if *negate {
                            !eq
                        } else {
                            eq
                        }
                    });
                }
                Step::StrNotGreaterThan { a, b } => {
                    rows.retain(|r| {
                        let (x, y) = (self.resolve(*a, r), self.resolve(*b, r));
                        match (dict.term(x), dict.term(y)) {
                            (oxrdf::Term::Literal(lx), oxrdf::Term::Literal(ly)) => {
                                lx.value() <= ly.value()
                            }
                            _ => false, // non-literal operand: the premise fails (engine `lex()` parity)
                        }
                    });
                }
                Step::UriToText {
                    iri,
                    out: o,
                    out_bound,
                } => {
                    let mut next = Vec::with_capacity(rows.len());
                    for mut row in rows {
                        let id = self.resolve(*iri, &row);
                        let oxrdf::Term::NamedNode(n) = dict.term(id) else {
                            continue;
                        };
                        let lit = dict.intern_lit(n.as_str(), XSD_STRING, None);
                        if self.unify_out(&mut row, *o, *out_bound, lit) {
                            next.push(row);
                        }
                    }
                    rows = next;
                }
                Step::TextToUri {
                    text,
                    out: o,
                    out_bound,
                } => {
                    let mut next = Vec::with_capacity(rows.len());
                    for mut row in rows {
                        let id = self.resolve(*text, &row);
                        let oxrdf::Term::Literal(l) = dict.term(id) else {
                            continue;
                        };
                        let iri = dict.intern_iri(l.value());
                        if self.unify_out(&mut row, *o, *out_bound, iri) {
                            next.push(row);
                        }
                    }
                    rows = next;
                }
                Step::EncodeForUri {
                    arg,
                    out: o,
                    out_bound,
                } => {
                    let mut next = Vec::with_capacity(rows.len());
                    for mut row in rows {
                        let id = self.resolve(*arg, &row);
                        let oxrdf::Term::Literal(l) = dict.term(id) else {
                            continue;
                        };
                        let enc = super::encode_for_uri(l.value());
                        let lit = dict.intern_lit(&enc, XSD_STRING, None);
                        if self.unify_out(&mut row, *o, *out_bound, lit) {
                            next.push(row);
                        }
                    }
                    rows = next;
                }
                Step::Concat {
                    args,
                    out: o,
                    out_bound,
                } => {
                    let mut next = Vec::with_capacity(rows.len());
                    'row: for mut row in rows {
                        let mut s = String::new();
                        for a in args {
                            let id = self.resolve(*a, &row);
                            if !concat_push(dict, id, &mut s) {
                                continue 'row;
                            }
                        }
                        let lit = dict.intern_lit(&s, XSD_STRING, None);
                        if self.unify_out(&mut row, *o, *out_bound, lit) {
                            next.push(row);
                        }
                    }
                    rows = next;
                }
                Step::Scrape {
                    arg,
                    regex,
                    out: o,
                    out_bound,
                } => {
                    let re = self.compiled.regexes[*regex].as_ref();
                    let mut next = Vec::with_capacity(rows.len());
                    for mut row in rows {
                        let Some(re) = re else { break }; // invalid regex: fails every row
                        let id = self.resolve(*arg, &row);
                        let oxrdf::Term::Literal(l) = dict.term(id) else {
                            continue;
                        };
                        let Some(cap) = re.captures(l.value()).and_then(|c| c.get(1)) else {
                            continue;
                        };
                        let lit = dict.intern_lit(cap.as_str(), XSD_STRING, None);
                        if self.unify_out(&mut row, *o, *out_bound, lit) {
                            next.push(row);
                        }
                    }
                    rows = next;
                }
            }
            if rows.is_empty() {
                return;
            }
        }
        for r in &rows {
            for c in &rule.conclusion {
                let g = [
                    self.resolve(c[0], r),
                    self.resolve(c[1], r),
                    self.resolve(c[2], r),
                ];
                if !store.all.contains(&g) {
                    out.push(g);
                }
            }
        }
    }

    /// Store-scoped negation as an ANTI-join: run the inner pattern seeded from the
    /// current rows through the SAME join machinery (full store, never the delta),
    /// reset this negation's wildcard slots on every surviving sub-row so it projects
    /// back onto the outer row it came from, and drop the outer rows that matched.
    fn anti_join(
        &self,
        rows: Vec<Row>,
        pats: &[PatternStep],
        wildcards: &[usize],
        store: &FactStore,
        width: usize,
    ) -> Vec<Row> {
        if rows.is_empty() {
            return rows;
        }
        let mut sub = rows.clone();
        for p in pats {
            let cands = self.candidates(p, store, None);
            sub = join_pattern(&sub, p, &cands, width);
            if sub.is_empty() {
                return rows; // nothing matches anywhere: notIncludes holds for every row
            }
        }
        let mut matched: FxHashSet<Row> = FxHashSet::default();
        for mut r in sub {
            for &w in wildcards {
                r[w] = NO_ID;
            }
            matched.insert(r);
        }
        rows.into_iter().filter(|r| !matched.contains(r)).collect()
    }
}

/// One pattern join: drive the SHARED substrate hash-join kernels — the binding rows are
/// the build side, the candidate facts the probe side — then reshape each combined row
/// (`bindings ++ [s, p, o]`) back into the full-width slot layout by writing the new
/// variables' columns (a repeated new variable within the atom becomes an equality
/// check). This is the reasoner's thin layout adapter over the generic kernels, the same
/// pattern as `crate::substrate_join` — no third join implementation.
fn join_pattern(rows: &[Row], p: &PatternStep, cands: &[Row], width: usize) -> Vec<Row> {
    if rows.is_empty() || cands.is_empty() {
        return Vec::new();
    }
    let keys = JoinKeys {
        key_cols: p.key_cols.clone(),
        right_only: Vec::new(),
    };
    let tables = vec![sjoin::build_table(rows, &keys)];
    let probe_only: Vec<usize> = vec![0, 1, 2];
    let mut combined: Vec<Row> = Vec::new();
    sjoin::hash_probe_serial(
        cands,
        &keys,
        rows,
        &tables,
        &probe_only,
        &NoBudget,
        &mut combined,
    );
    let mut out = Vec::with_capacity(combined.len());
    'row: for c in &combined {
        let (b, f) = c.split_at(width);
        let mut row = Row::from_slice(b);
        for &(v, i) in &p.new_writes {
            if row[v] != NO_ID && row[v] != f[i] {
                continue 'row; // repeated new variable in one atom must agree
            }
            row[v] = f[i];
        }
        out.push(row);
    }
    out
}

/// `string:concatenation` argument coercion, mirroring the text engine's: IRIs coerce
/// to their text, typed literals to their canonical VALUE string (`"0"^^xsd:boolean` →
/// `"false"`, `"07"^^xsd:integer` → `"7"`), everything else to its lexical form; a
/// blank-node argument fails the premise. Kept in lock-step with the `Func::Concat` arm
/// of the text engine's `eval_functional` (same `numval`/`dec_norm`/`numval_term`
/// helpers) — the equivalence suite is the drift alarm.
fn concat_push(dict: &Dict, id: Id, s: &mut String) -> bool {
    match dict.term(id) {
        oxrdf::Term::NamedNode(n) => {
            s.push_str(n.as_str());
            true
        }
        oxrdf::Term::Literal(l) => {
            let v = l.value();
            match l
                .datatype()
                .as_str()
                .strip_prefix("http://www.w3.org/2001/XMLSchema#")
            {
                Some("boolean") => {
                    s.push_str(if v == "0" || v == "false" {
                        "false"
                    } else {
                        "true"
                    });
                }
                Some("integer" | "decimal" | "float" | "double") => {
                    let t = Term::Lit(v.to_string(), l.datatype().as_str().to_string(), None);
                    match super::numval(&t) {
                        Some(super::NumVal::Int(i)) => s.push_str(&i.to_string()),
                        Some(super::NumVal::Dec(m, sc)) => {
                            let (m, sc) = super::dec_norm(m, sc);
                            if sc == 0 {
                                s.push_str(&m.to_string());
                            } else if let Term::Lit(lex, ..) =
                                super::numval_term(super::NumVal::Dec(m, sc))
                            {
                                s.push_str(&lex);
                            }
                        }
                        Some(super::NumVal::F64(f)) => {
                            if f.fract() == 0.0 && f.abs() < 9.007e15 {
                                let _ = std::fmt::Write::write_fmt(s, format_args!("{}", f as i64));
                            } else {
                                let _ = std::fmt::Write::write_fmt(s, format_args!("{f}"));
                            }
                        }
                        None => s.push_str(v),
                    }
                }
                _ => s.push_str(v),
            }
            true
        }
        _ => false, // blank node / triple term argument: the premise fails (engine parity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;

    /// The closure of `facts_src` + `rules_src` through BOTH engines, as sets of
    /// N-Triples-formatted strings (dictionary-independent), asserted EQUAL.
    fn assert_equivalent(facts_src: &str, rules_src: &str) -> FxHashSet<String> {
        let text = closure_text(facts_src, rules_src);
        let compiled = closure_compiled(facts_src, rules_src);
        let missing: Vec<_> = text.difference(&compiled).collect();
        let extra: Vec<_> = compiled.difference(&text).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "compiled closure diverges from reason_n3:\n  missing ({}): {missing:?}\n  extra ({}): {extra:?}",
            missing.len(),
            extra.len()
        );
        compiled
    }

    fn closure_text(facts_src: &str, rules_src: &str) -> FxHashSet<String> {
        let mut dict = Dict::new();
        let ids = crate::reason_n3(&mut dict, &format!("{facts_src}\n{rules_src}")).unwrap();
        triples_as_strings(&dict, &ids)
    }

    fn closure_compiled(facts_src: &str, rules_src: &str) -> FxHashSet<String> {
        let rules = compile(rules_src).unwrap();
        let mut dict = Dict::new();
        let facts = intern_facts(&mut dict, facts_src).unwrap();
        let ids = eval(&mut dict, &facts, &rules);
        triples_as_strings(&dict, &ids)
    }

    fn triples_as_strings(dict: &Dict, ids: &[[Id; 3]]) -> FxHashSet<String> {
        ids.iter()
            .map(|t| {
                format!(
                    "{} {} {}",
                    dict.term(t[0]),
                    dict.term(t[1]),
                    dict.term(t[2])
                )
            })
            .collect()
    }

    fn has(set: &FxHashSet<String>, s: &str, p: &str, o: &str) -> bool {
        set.contains(&format!("<{s}> <{p}> <{o}>"))
    }

    #[test]
    fn compile_counts_rules_and_doc_facts() {
        let rules = compile(
            "@prefix : <http://ex/> .\n:a :mode :b .\n{ ?x a :Man } => { ?x a :Mortal } .\n{ ?x a :Mortal } => { ?x a :Being } .",
        )
        .unwrap();
        assert_eq!(rules.n_rules(), 2);
        assert_eq!(rules.n_facts(), 1);
    }

    #[test]
    fn simple_rule_matches_reason_n3() {
        let s = assert_equivalent(
            "@prefix : <http://ex/> . :Socrates a :Man .",
            "@prefix : <http://ex/> . { ?x a :Man } => { ?x a :Mortal } .",
        );
        assert!(has(
            &s,
            "http://ex/Socrates",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            "http://ex/Mortal"
        ));
    }

    #[test]
    fn recursive_ancestor_chain_semi_naive() {
        // A 30-deep parent chain: exercises the delta rounds of the semi-naive fixpoint.
        let mut facts = String::from("@prefix : <http://ex/> .\n");
        for i in 0..30 {
            facts.push_str(&format!(":n{i} :parent :n{} .\n", i + 1));
        }
        let rules = "@prefix : <http://ex/> .\n\
             { ?r :parent ?p . } => { ?r :ancestor ?p } .\n\
             { ?r :parent ?p . ?p :ancestor ?a . } => { ?r :ancestor ?a } .";
        let s = assert_equivalent(&facts, rules);
        assert!(has(
            &s,
            "http://ex/n0",
            "http://ex/ancestor",
            "http://ex/n30"
        ));
    }

    #[test]
    fn rule_document_facts_enter_the_closure() {
        // The rule doc carries plain facts (the acp-c.n3 mode-mapping shape).
        let s = assert_equivalent(
            "@prefix : <http://ex/> . :pol :allow :Read . :k :satisfied true .",
            "@prefix : <http://ex/> .\n:Read :allowPred :read .\n\
             { ?pol :allow ?m . ?m :allowPred ?pred . ?k :satisfied true . } => { :alice ?pred :r } .",
        );
        assert!(has(
            &s,
            "http://ex/Read",
            "http://ex/allowPred",
            "http://ex/read"
        ));
        assert!(has(&s, "http://ex/alice", "http://ex/read", "http://ex/r"));
    }

    #[test]
    fn not_includes_store_scoped_naf() {
        // Nearest-ancestor shape: r1 has its own ACL, r2 does not.
        let facts = "@prefix : <http://ex/> .\n\
             :r1 :parent :p . :r2 :parent :p . :p :ownAcl :acl .\n:r1 :ownAcl :acl1 .";
        let rules =
            "@prefix : <http://ex/> . @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             { ?r :parent ?p . ?p :ownAcl ?acl . ?S log:notIncludes { ?r :ownAcl ?x . } . }\n\
             => { ?r :inheritedAcl ?acl } .";
        let s = assert_equivalent(facts, rules);
        assert!(has(
            &s,
            "http://ex/r2",
            "http://ex/inheritedAcl",
            "http://ex/acl"
        ));
        assert!(!has(
            &s,
            "http://ex/r1",
            "http://ex/inheritedAcl",
            "http://ex/acl"
        ));
    }

    #[test]
    fn not_includes_multi_atom_body_with_shared_wildcard() {
        // The ODRL-spike prohibition carve-out shape: a wildcard (?px) shared across
        // the negation body's atoms, correlated with outer-bound vars.
        let facts = "@prefix : <http://ex/> .\n\
             :pol :permission :perm1 . :perm1 :action :read . :perm1 :target :t1 .\n\
             :pol :permission :perm2 . :perm2 :action :read . :perm2 :target :t2 .\n\
             :pol :prohibition :px1 . :px1 :action :read . :px1 :target :t1 .";
        let rules = "@prefix : <http://ex/> . @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             { ?pol :permission ?perm . ?perm :action ?act . ?perm :target ?tgt .\n\
               ?S log:notIncludes { ?pol :prohibition ?px . ?px :action ?act . ?px :target ?tgt . } . }\n\
             => { ?tgt :granted ?act } .";
        let s = assert_equivalent(facts, rules);
        assert!(has(
            &s,
            "http://ex/t2",
            "http://ex/granted",
            "http://ex/read"
        ));
        assert!(!has(
            &s,
            "http://ex/t1",
            "http://ex/granted",
            "http://ex/read"
        ));
    }

    #[test]
    fn log_uri_both_directions_and_minting() {
        // The wac.n3 pair-principal mint: log:uri forward, encodeForUri, concatenation,
        // log:uri reverse — the whole string tower in one rule.
        let facts = "@prefix : <http://ex/> .\n:auth :agent <https://alice.ex/card#me> . :auth :origin <https://app.ex> .";
        let rules = "@prefix : <http://ex/> .\n\
             @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n\
             { ?auth :agent ?a . ?auth :origin ?o . ?a log:uri ?as . ?o log:uri ?os .\n\
               ?as string:encodeForUri ?ae . ?os string:encodeForUri ?oe .\n\
               (\"urn:sparq:pair?agent=\" ?ae \"&client=\" ?oe) string:concatenation ?ps . ?p log:uri ?ps . }\n\
             => { ?p :minted true } .";
        let s = assert_equivalent(facts, rules);
        let expected = format!(
            "urn:sparq:pair?agent={}&client={}",
            crate::n3::encode_for_uri("https://alice.ex/card#me"),
            crate::n3::encode_for_uri("https://app.ex")
        );
        assert!(
            s.iter().any(|t| t.starts_with(&format!("<{expected}>"))),
            "expected minted principal <{expected}> in {s:?}"
        );
    }

    #[test]
    fn concatenation_coerces_typed_literals() {
        // Non-canonical integer + boolean + decimal coercion (engine value semantics).
        let s = assert_equivalent(
            "@prefix : <http://ex/> . :s :p \"07\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "@prefix : <http://ex/> . @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n\
             { ?s :p ?v . (\"n=\" ?v \"/\" \"0\"^^<http://www.w3.org/2001/XMLSchema#boolean> \"/\" \"2.50\"^^<http://www.w3.org/2001/XMLSchema#decimal>) string:concatenation ?out . }\n\
             => { ?s :out ?out } .",
        );
        assert!(
            s.contains("<http://ex/s> <http://ex/out> \"n=7/false/2.5\""),
            "typed-literal coercion diverged: {s:?}"
        );
    }

    #[test]
    fn scrape_and_parent_walk() {
        // The common.n3 parent-derivation shape: log:uri + string:scrape + log:uri.
        let facts = "@prefix solidx: <https://sparq.dev/ns/solidx#> .\n\
             <https://pod.ex/a/b.ttl> solidx:isResource true .\n\
             <https://pod.ex/a/> solidx:isResource true .";
        let rules = "@prefix solidx: <https://sparq.dev/ns/solidx#> .\n\
             @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n\
             @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             { ?r solidx:isResource true . ?r log:uri ?rs .\n\
               (?rs \"^(.*/)[^/]+/?$\") string:scrape ?ps . ?p log:uri ?ps . }\n\
             => { ?r solidx:parentCand ?p } .\n\
             { ?r solidx:parentCand ?p . ?p solidx:isResource true . } => { ?r solidx:parent ?p } .";
        let s = assert_equivalent(facts, rules);
        assert!(has(
            &s,
            "https://pod.ex/a/b.ttl",
            "https://sparq.dev/ns/solidx#parent",
            "https://pod.ex/a/"
        ));
    }

    #[test]
    fn comparisons_not_equal_and_str_not_greater() {
        let facts = "@prefix : <http://ex/> .\n\
             :m :client :c1 . :m :client :Public .\n\
             :req :at \"2026-01-01T00:00:00Z\" . :req2 :at \"2027-01-01T00:00:00Z\" .";
        let rules = "@prefix : <http://ex/> . @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n\
             { ?m :client ?c . ?c log:notEqualTo :Public . } => { ?m :concrete ?c } .\n\
             { ?r :at ?t . ?t string:notGreaterThan \"2026-06-01T00:00:00Z\" . } => { ?r :inWindow true } .";
        let s = assert_equivalent(facts, rules);
        assert!(has(&s, "http://ex/m", "http://ex/concrete", "http://ex/c1"));
        assert!(!has(
            &s,
            "http://ex/m",
            "http://ex/concrete",
            "http://ex/Public"
        ));
        assert!(s.contains("<http://ex/req> <http://ex/inWindow> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"));
        assert!(!s
            .iter()
            .any(|t| t.starts_with("<http://ex/req2> <http://ex/inWindow>")));
    }

    #[test]
    fn variable_predicate_join() {
        // The acp-a provenance shape: a variable in predicate position, bound by data.
        let s = assert_equivalent(
            "@prefix : <http://ex/> . :m :provMatcher :creator . :r :creator :w .",
            "@prefix : <http://ex/> . { ?m :provMatcher ?kind . ?r ?kind ?w . } => { ?m :provAgent ?w } .",
        );
        assert!(has(&s, "http://ex/m", "http://ex/provAgent", "http://ex/w"));
    }

    #[test]
    fn pure_constant_rule_fires_once() {
        let s = assert_equivalent(
            "@prefix : <http://ex/> . :seed :is true .",
            "@prefix : <http://ex/> . {} => { :a :b :c } .",
        );
        assert!(has(&s, "http://ex/a", "http://ex/b", "http://ex/c"));
    }

    // -----------------------------------------------------------------------
    // RDF 1.2 / RDF-star quoted triples (sq-6d43t)
    // -----------------------------------------------------------------------

    #[test]
    fn ground_quoted_triple_is_a_plain_constant() {
        // A ground `<< … >>` is an ordinary symbol-table constant: it interns to the
        // dictionary's content-addressed triple-term id, so it works as a fact term, a
        // candidate pre-filter in a pattern, and a conclusion term.
        let s = assert_equivalent(
            "@prefix : <http://ex/> . << :a :p :b >> :says :alice . << :a :p :c >> :says :bob .",
            "@prefix : <http://ex/> . { << :a :p :b >> :says ?w } => { ?w :vouches :a . :meta :about << :a :p :b >> } .",
        );
        assert!(has(&s, "http://ex/alice", "http://ex/vouches", "http://ex/a"));
        assert!(
            !has(&s, "http://ex/bob", "http://ex/vouches", "http://ex/a"),
            "the ground quotation must filter candidates: {s:?}"
        );
        assert!(
            s.iter().any(|t| t.starts_with("<http://ex/meta> <http://ex/about> <<")),
            "a ground quoted triple must be derivable as a conclusion constant: {s:?}"
        );
    }

    #[test]
    fn ground_quoted_triple_is_a_builtin_argument() {
        // "a ground constant ANYWHERE" includes a builtin's operand — `log:notEqualTo`
        // compares the two ids, and the quotation's id is just another symbol.
        let s = assert_equivalent(
            "@prefix : <http://ex/> . << :a :p :b >> :says :alice . << :a :p :c >> :says :bob .",
            "@prefix : <http://ex/> . @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             { ?q :says ?w . ?q log:notEqualTo << :a :p :c >> . } => { ?w :nonMatching true } .",
        );
        assert!(s.contains("<http://ex/alice> <http://ex/nonMatching> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"));
        assert!(
            !s.iter().any(|t| t.starts_with("<http://ex/bob> <http://ex/nonMatching>")),
            "the quoted-triple operand must compare equal to its own id: {s:?}"
        );
    }

    #[test]
    fn quoted_triple_pattern_binds_inner_variables() {
        // The component-indexed match: the join binds the quoted TERM id, the unpack
        // step decomposes it into component ids and binds ?s / ?o from them.
        let s = assert_equivalent(
            "@prefix : <http://ex/> . << :a :p :b >> :says :alice . :a :p :b . << :a :q :d >> :says :bob .",
            "@prefix : <http://ex/> . { << ?s :p ?o >> :says ?w } => { ?s :linked ?o . ?w :vouches ?s } .",
        );
        assert!(has(&s, "http://ex/a", "http://ex/linked", "http://ex/b"));
        assert!(has(&s, "http://ex/alice", "http://ex/vouches", "http://ex/a"));
        // The `:q` quotation does not match the `:p` component constant, and the plain
        // `:a :p :b` triple is not a triple TERM, so neither fires the rule.
        assert!(
            !has(&s, "http://ex/bob", "http://ex/vouches", "http://ex/a"),
            "constant component must filter: {s:?}"
        );
    }

    #[test]
    fn quoted_triple_pattern_correlates_with_outer_bindings() {
        // ?s / ?o are bound by a PRECEDING plain atom, so the unpack step must CHECK
        // them against the components rather than rebind — the join direction the
        // access-control shape ("is this asserted triple vouched for?") needs.
        let s = assert_equivalent(
            "@prefix : <http://ex/> .\n\
             :a :p :b . :a :p :c .\n\
             << :a :p :b >> :assertedBy :alice .",
            "@prefix : <http://ex/> . { ?s :p ?o . << ?s :p ?o >> :assertedBy ?w . } => { ?o :sourced ?w } .",
        );
        assert!(has(&s, "http://ex/b", "http://ex/sourced", "http://ex/alice"));
        assert!(
            !has(&s, "http://ex/c", "http://ex/sourced", "http://ex/alice"),
            "only the quoted-AND-asserted triple is sourced — the unpack must CHECK the \
             already-bound ?s/?o against the components, not wave them through: {s:?}"
        );
    }

    #[test]
    fn quoted_triple_repeated_inner_variable() {
        // `<< ?x :p ?x >>` binds on the subject component and FILTERS on the object —
        // the left-to-right component order the text engine unifies in.
        let s = assert_equivalent(
            "@prefix : <http://ex/> . << :a :p :a >> :says :alice . << :a :p :b >> :says :bob .",
            "@prefix : <http://ex/> . { << ?x :p ?x >> :says ?w } => { ?x :selfAsserted ?w } .",
        );
        assert!(has(
            &s,
            "http://ex/a",
            "http://ex/selfAsserted",
            "http://ex/alice"
        ));
        assert!(
            !s.iter().any(|t| t.contains("<http://ex/selfAsserted> <http://ex/bob>")),
            "the repeated inner variable must filter: {s:?}"
        );
    }

    #[test]
    fn nested_quoted_triple_pattern() {
        // Two levels of quotation, nested through the OBJECT (the only nesting RDF 1.2
        // triple terms admit): the outer unpack binds the inner triple TERM id into a
        // fresh slot, and a second unpack step decomposes that.
        let s = assert_equivalent(
            "@prefix : <http://ex/> .\n\
             :m1 :meta << :a :r << :b :q :c >> >> .\n\
             :m2 :meta << :a :r << :b :q :d >> >> .",
            "@prefix : <http://ex/> . { ?m :meta << ?a :r << ?b :q :c >> >> } => { ?m :outer ?a . ?m :inner ?b } .",
        );
        assert!(has(&s, "http://ex/m1", "http://ex/outer", "http://ex/a"));
        assert!(has(&s, "http://ex/m1", "http://ex/inner", "http://ex/b"));
        assert!(
            !s.iter().any(|t| t.starts_with("<http://ex/m2> <http://ex/inner>")),
            "the innermost object constant must filter the nested match: {s:?}"
        );
    }

    #[test]
    fn quoted_triple_in_subject_and_object_positions() {
        // A quotation in SUBJECT position of the atom (the common annotation shape) and
        // one in OBJECT position, in the same rule.
        let s = assert_equivalent(
            "@prefix : <http://ex/> . << :a :p :b >> :because << :c :q :d >> .",
            "@prefix : <http://ex/> . { << ?s :p ?o >> :because << ?s2 :q ?o2 >> } => { ?s :chain ?o2 . ?s2 :grounds ?o } .",
        );
        assert!(has(&s, "http://ex/a", "http://ex/chain", "http://ex/d"));
        assert!(has(&s, "http://ex/c", "http://ex/grounds", "http://ex/b"));
    }

    #[test]
    fn quoted_pattern_does_not_match_an_asserted_triple() {
        // RDF-star OPACITY, at the id level: `<< ?s ?p ?o >>` matches a triple TERM,
        // never a plain asserted triple. With every component a variable, the only thing
        // standing between the two is the unpack step's term-KIND check.
        let s = assert_equivalent(
            "@prefix : <http://ex/> . :a :p :b . :a :says :alice .",
            "@prefix : <http://ex/> . { << ?s ?p ?o >> :says ?w } => { ?s :fromQuote ?o } .",
        );
        assert!(
            !s.iter().any(|t| t.contains("<http://ex/fromQuote>")),
            "a quoted pattern fired on an asserted triple: {s:?}"
        );
    }

    #[test]
    fn quoted_triple_rule_is_semi_naive_over_derived_facts() {
        // The quoted-triple atom is an ordinary delta position: a fact DERIVED in round
        // 1 must still drive the quotation match in round 2.
        let s = assert_equivalent(
            "@prefix : <http://ex/> . :seed :trusts :alice . << :a :p :b >> :says :alice .",
            "@prefix : <http://ex/> .\n\
             { :seed :trusts ?w } => { ?w :active true } .\n\
             { << ?s :p ?o >> :says ?w . ?w :active true . } => { ?s :accepted ?o } .",
        );
        assert!(has(&s, "http://ex/a", "http://ex/accepted", "http://ex/b"));
    }

    #[test]
    fn intern_facts_accepts_ground_quoted_triples() {
        let mut dict = Dict::new();
        let facts = intern_facts(
            &mut dict,
            "@prefix : <http://ex/> . << :a :p :b >> :says :alice .",
        )
        .unwrap();
        assert_eq!(facts.len(), 1);
        // The subject id is the dictionary's content-addressed triple term, and it
        // decomposes back into exactly the component ids.
        let comps = triple_components(&dict, facts[0][0]).expect("subject is a triple term");
        assert_eq!(
            comps,
            [
                dict.intern_iri("http://ex/a"),
                dict.intern_iri("http://ex/p"),
                dict.intern_iri("http://ex/b"),
            ]
        );
        // …and a NON-triple id decomposes to None (the unpack step's row filter).
        assert!(triple_components(&dict, facts[0][2]).is_none());
        assert!(triple_components(&dict, NO_ID).is_none());
    }

    #[test]
    fn quoted_triple_shapes_outside_the_subset_error_loudly() {
        // Variables inside a quotation in a CONCLUSION: minting can violate RDF 1.2's
        // structural constraints at derivation time, with no channel to report it.
        assert!(compile(
            "@prefix : <http://ex/> . { ?x :p ?y } => { << ?x :p ?y >> :derivedFrom :rule1 } ."
        )
        .unwrap_err()
        .contains("in a conclusion is not in the compiled subset"));
        // …but a GROUND quotation in a conclusion is fine.
        assert!(compile(
            "@prefix : <http://ex/> . { ?x :p ?y } => { << :a :p :b >> :derivedFrom :rule1 } ."
        )
        .is_ok());
        // Variables inside a quotation in a log:notIncludes body: the anti-join runs a
        // flat list of plain patterns.
        assert!(compile(
            "@prefix : <http://ex/> . @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
             { ?x :p ?y . ?S log:notIncludes { << ?a :q ?b >> :says ?x . } . } => { ?x :ok true } ."
        )
        .unwrap_err()
        .contains("log:notIncludes body"));
        // A quoted FORMULA inside a quotation is still outside the subset.
        assert!(compile(
            "@prefix : <http://ex/> . { << ?s :p { :a :b :c } >> :says ?w } => { ?s :ok true } ."
        )
        .unwrap_err()
        .contains("not a compilable constant"));
        // A quoted triple whose subject is a literal has no RDF 1.2 representation.
        assert!(compile(
            "@prefix : <http://ex/> . { << \"lit\" :p :o >> :says ?w } => { ?w :ok true } ."
        )
        .unwrap_err()
        .contains("is not an IRI or blank node"));
        // A quotation NESTED IN SUBJECT POSITION: the text engine matches it at the term
        // level, but no dictionary triple term can have a triple-term subject, so the
        // id-level path says so at compile time instead of never firing.
        assert!(compile(
            "@prefix : <http://ex/> . { << << ?x :q ?y >> :r ?z >> :meta ?m } => { ?x :ok ?y } ."
        )
        .unwrap_err()
        .contains("no other subject kind"));
        // Same for a variable predicate's sibling: a literal in predicate position.
        assert!(compile(
            "@prefix : <http://ex/> . { << ?s \"lit\" ?o >> :says ?w } => { ?w :ok true } ."
        )
        .unwrap_err()
        .contains("no other predicate kind"));
    }

    #[test]
    fn unsupported_constructs_error_loudly() {
        // Backward rule.
        assert!(
            compile("@prefix : <http://ex/> . { ?x a :Man } <= { ?x a :Human } .")
                .unwrap_err()
                .contains("backward")
        );
        // Unsupported functional builtin.
        assert!(compile(
            "@prefix : <http://ex/> . @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n{ ?x :p ?a . (?a 1) math:sum ?b . } => { ?x :q ?b } ."
        )
        .unwrap_err()
        .contains("not in the compiled subset"));
        // Unsupported comparison builtin.
        assert!(compile(
            "@prefix : <http://ex/> . @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n{ ?x :p ?a . ?a math:greaterThan 1 . } => { ?x :big true } ."
        )
        .unwrap_err()
        .contains("not in the compiled subset"));
        // List generator.
        assert!(compile(
            "@prefix : <http://ex/> . @prefix list: <http://www.w3.org/2000/10/swap/list#> .\n{ ?x :p ?l . ?l list:member ?m . } => { ?x :has ?m } ."
        )
        .unwrap_err()
        .contains("list generator"));
        // log:includes (only notIncludes is scoped-supported).
        assert!(compile(
            "@prefix : <http://ex/> . @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n{ ?S log:includes { :a :b :c } . } => { :s :ok true } ."
        )
        .unwrap_err()
        .contains("log:includes"));
        // Conclusion existential blank.
        assert!(
            compile("@prefix : <http://ex/> . { ?x a :Man } => { ?x :knows _:someone } .")
                .unwrap_err()
                .contains("existential blank")
        );
        // Unbound conclusion variable.
        assert!(
            compile("@prefix : <http://ex/> . { ?x a :Man } => { ?x :knows ?y } .")
                .unwrap_err()
                .contains("not bound by the premise")
        );
        // Unresolvable builtin input.
        assert!(compile(
            "@prefix : <http://ex/> . @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n{ ?u string:encodeForUri ?e . } => { :s :enc ?e } ."
        )
        .unwrap_err()
        .contains("not bound by any preceding pattern"));
    }

    #[test]
    fn intern_facts_rejects_rule_documents() {
        let mut dict = Dict::new();
        let err = intern_facts(
            &mut dict,
            "@prefix : <http://ex/> . { ?x a :A } => { ?x a :B } .",
        )
        .unwrap_err();
        assert!(err.contains("contains rules"));
    }

    #[test]
    fn bind_interns_the_symbol_table_into_the_caller_dict() {
        let rules =
            compile("@prefix : <http://ex/> . { ?x a :Man } => { ?x a :Mortal } .").unwrap();
        let mut dict = Dict::new();
        let bound = rules.bind(&mut dict);
        // The rule vocabulary is now resolvable in the caller's dictionary.
        let mortal = dict.lookup(&oxrdf::Term::NamedNode(NamedNode::new_unchecked(
            "http://ex/Mortal",
        )));
        assert_ne!(
            mortal, 0,
            "bind() must intern rule constants into the caller Dict"
        );
        // And the bound set evaluates against ids from that same dictionary.
        let man = dict.intern_iri("http://ex/Man");
        let ty = dict.intern_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let socrates = dict.intern_iri("http://ex/Socrates");
        let closure = bound.eval(&mut dict, &[[socrates, ty, man]]);
        assert!(closure.contains(&[socrates, ty, mortal]));
    }

    #[test]
    fn repeated_variable_within_one_atom() {
        let s = assert_equivalent(
            "@prefix : <http://ex/> . :a :p :a . :a :p :b .",
            "@prefix : <http://ex/> . { ?x :p ?x . } => { ?x :selfLoop true } .",
        );
        assert!(s.contains("<http://ex/a> <http://ex/selfLoop> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"));
        assert!(!s
            .iter()
            .any(|t| t.starts_with("<http://ex/b> <http://ex/selfLoop>")));
    }
}
