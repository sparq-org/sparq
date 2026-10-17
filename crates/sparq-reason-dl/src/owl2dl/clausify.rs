//! OWL 2 axioms → DL-clauses, the input of the [hypertableau](super::hypertableau).
//!
//! The translation follows Motik, Shearer and Horrocks, "Hypertableau Reasoning for
//! Description Logics" (JAIR 36, 2009), §4:
//!
//! 1. every class axiom becomes one or more GCIs `⊤ ⊑ C` with `C` in negation normal form;
//! 2. the structural transformation names every non-literal subconcept with a fresh atomic
//!    concept, so each GCI is a disjunction of *literal* concepts (`A`, `¬A`, `∀R.A`,
//!    `≥n R.A`, `≤n R.A`, `∃R.Self`, `¬∃R.Self`);
//! 3. each such disjunction is read as a DL-clause `U₁ ∧ … ∧ Uₘ → V₁ ∨ … ∨ Vₙ` over a central
//!    variable `x` and neighbour variables `yᵢ`.
//!
//! Property chains and transitivity are eliminated by encoding (Horrocks and Sattler,
//! "Decidability of SHIQ with complex role inclusion axioms", AIJ 160, 2004; Motik et al.
//! §5): every non-simple role `R` gets a finite automaton accepting the role words that imply
//! `R`, and `∀R.C` becomes one fresh concept `X_q` per automaton state, with
//! `X_q(x) ∧ S(x, y) → X_q'(y)` per transition, `X_q ⊑ X_q'` per ε-transition and
//! `X_f ⊑ C` for the final state; so the tableau never sees a chain. The role hierarchy is
//! not translated to clauses: the tableau materialises super-role facts directly.
//!
//! Data properties are roles too (never inverted, never transitive) whose edges end in
//! *concrete* nodes, and each data range becomes a [`DataRangeExpr`] atom
//! ([`AtomKind::Data`]) that only ever labels concrete nodes and only ever occurs in clause
//! heads: `∀D.R` is `D(x, y) → R(y)`, `≥n D.R` an at-least atom with an `R` filler, and
//! `≤n D.R` the clause `D(x, y₀) ∧ … ∧ D(x, yₙ) → ⋁ ¬R(yᵢ) ∨ ⋁ yᵢ ≈ yⱼ` (a concrete node is
//! either outside `R` or merged; `¬R` is again a data range). `DisjointDataProperties(D E)`
//! is `D(x, y₀) ∧ E(x, y₁) → y₀ ≉ y₁`, an inequality between *values*. Named datatypes
//! defined by a `DatatypeDefinition` are expanded (refused when the definitions are cyclic
//! or a datatype has more than one). The tableau's datatype check decides the labels with
//! [`super::data_ranges`].
//!
//! Everything outside the supported fragment (keys on non-simple roles, the universal
//! roles, datatypes outside the datatype map, an irregular or too large role hierarchy) is
//! refused with [`Unsupported`], naming the axiom's source triples, so the reasoner can answer `Unknown(OutOfFragment)`
//! instead of a wrong verdict; a data range or literal the datatype oracle cannot decide is
//! refused as a datatype question (`Unknown(DatatypeUnknown)`).

use super::data_ranges::{self, DataRangeExpr, Tri};
use super::datatypes::{self, builtin, Builtin, Value, ValueError};
use super::dl_profile::canonical_range;
use super::model::{
    Axiom, ClassExpression, DataRange, EntityKind, Individual, Literal, ObjectProperty, Ontology,
};
use super::vocab::{iri_of, Vocab, RDF};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};
use std::cell::Cell;

/// A role expression: a named object property (by internal index) or its inverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Role {
    pub idx: u32,
    pub inv: bool,
}

impl Role {
    pub(crate) fn inverse(self) -> Role {
        Role {
            idx: self.idx,
            inv: !self.inv,
        }
    }
}

/// An atomic concept of the clause set: an index into [`Clauses::atoms`].
pub(crate) type Atom = u32;

/// What an [`Atom`] stands for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AtomKind {
    /// A named class of the ontology.
    Class(Id),
    /// A fresh name from the structural transformation (or an ABox marker).
    Fresh,
    /// The at-least concept `≥n R.F` (`F = None` is `⊤`). For a data role `R` the filler
    /// is a [`AtomKind::Data`] atom and the successors are concrete nodes.
    AtLeast {
        n: u32,
        role: Role,
        filler: Option<Atom>,
    },
    /// A data range, by index into [`Clauses::data`]. Labels concrete nodes only.
    Data(u32),
    /// The nominal `{a}` of the individual with this index: the node it labels is `a`'s
    /// node (the tableau merges the two).
    Nominal(u32),
}

/// A variable of a DL-clause: the central `x` or the `i`-th neighbour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Var {
    X,
    Y(u16),
}

/// A head atom of a DL-clause.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Head {
    /// `A(v)`; at-least concepts are atoms too.
    Concept(Atom, Var),
    /// `u ≈ v`.
    Eq(Var, Var),
    /// `R(x, x)`.
    SelfLoop(Role),
    /// `u ≉ v` between concrete nodes: their values differ (`DisjointDataProperties`).
    Ineq(Var, Var),
}

/// A DL-clause. Every neighbour variable `yᵢ` occurs in at least one role atom, so a match
/// is found by walking the edges of the central node.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Clause {
    /// `A(x)` body atoms.
    pub body_x: Vec<Atom>,
    /// `R(x, yᵢ)` body atoms (`R` may be an inverse, i.e. `S(yᵢ, x)`).
    pub body_roles: Vec<(Role, u16)>,
    /// `A(yᵢ)` body atoms.
    pub body_y: Vec<(Atom, u16)>,
    /// `R(x, x)` body atoms.
    pub body_self: Vec<Role>,
    /// Number of neighbour variables.
    pub n_y: u16,
    /// Neighbour variables `yᵢ` interchangeable with `yᵢ₋₁` (an at-most restriction's
    /// successors): a match may bind them in increasing node order only.
    pub ordered: Vec<u16>,
    /// The disjunctive head; empty means `⊥`.
    pub head: Vec<Head>,
    /// The source triples of the axiom this clause came from.
    pub source: u32,
}

/// The ABox, in terms of [`Atom`]s and [`Role`]s over individual indices.
#[derive(Clone, Debug, Default)]
pub(crate) struct Abox {
    /// The individuals, in first-mention order; their index is their node id.
    pub individuals: Vec<Individual>,
    /// `A(a)` facts, with the axiom index they came from.
    pub concepts: Vec<(Atom, u32, u32)>,
    /// `R(a, b)` facts.
    pub roles: Vec<(Role, u32, u32, u32)>,
    /// `a ≈ b` facts.
    pub same: Vec<(u32, u32, u32)>,
    /// `a ≉ b` facts.
    pub different: Vec<(u32, u32, u32)>,
    /// `D(a, v)` facts: the data role, the individual, the `OneOf(v)` atom and the value.
    pub data: Vec<(Role, u32, Atom, Value, u32)>,
}

/// A `HasKey` axiom: two named individuals in the class that share a named value on every
/// object property and a value on every data property of the key are the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    /// An atom equivalent to the key's class (`None`: `owl:Thing`).
    pub class: Option<Atom>,
    /// The object property expressions of the key.
    pub roles: Vec<Role>,
    /// The data properties of the key.
    pub data: Vec<Role>,
}

/// A construct outside the fragment the reasoner decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// The construct, e.g. `"ObjectOneOf"`.
    pub construct: &'static str,
    /// The index of the axiom in [`Ontology::axioms`].
    pub axiom: usize,
    pub kind: Refusal,
}

/// Why [`clausify`] gave no clauses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The construct is outside the fragment.
    Fragment,
    /// The construct is in the fragment but the datatype oracle cannot decide it (a data
    /// range or literal it answers Unknown for).
    Datatype,
    /// The translation ran out of the work budget (datatype definitions expanding to more
    /// than it allows).
    Budget,
}

/// The ontology as DL-clauses plus its ABox and role hierarchy.
#[derive(Clone, Debug, Default)]
pub(crate) struct Clauses {
    pub atoms: Vec<AtomKind>,
    pub clauses: Vec<Clause>,
    pub abox: Abox,
    /// The named object properties, by internal role index.
    pub roles: Vec<Id>,
    /// For each role expression (`2·idx + inv`), every super-role expression including itself.
    pub supers: Vec<Vec<Role>>,
    /// The named classes of the ontology's signature, by atom.
    pub classes: Vec<(Id, Atom)>,
    /// Told named-class subsumptions `A ⊑ B` (from `SubClassOf` and `EquivalentClasses`).
    pub told: FxHashSet<(Id, Id)>,
    /// The data ranges of the [`AtomKind::Data`] atoms.
    pub data: Vec<DataRangeExpr>,
    /// The axiom (index into [`Ontology::axioms`]) each data range was first read from.
    pub data_source: Vec<u32>,
    /// Per role index: whether it is a data property.
    pub data_roles: Vec<bool>,
    /// The [`AtomKind::Nominal`] atoms, by individual index. Non-empty makes class tests
    /// depend on the ABox.
    pub nominals: FxHashMap<u32, Atom>,
    /// The `HasKey` axioms.
    pub keys: Vec<Key>,
}

impl Clauses {
    pub(crate) fn supers(&self, r: Role) -> &[Role] {
        &self.supers[(2 * r.idx + u32::from(r.inv)) as usize]
    }

    /// Whether `r` is a data property (its edges end in concrete nodes).
    pub(crate) fn is_data(&self, r: Role) -> bool {
        self.data_roles[r.idx as usize]
    }
}

/// A concept in negation normal form over [`Atom`]s.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Nnf {
    Top,
    Bottom,
    Atom(Atom),
    Not(Atom),
    And(Vec<Nnf>),
    Or(Vec<Nnf>),
    All(Role, Box<Nnf>),
    AtLeast(u32, Role, Box<Nnf>),
    AtMost(u32, Role, Box<Nnf>),
    SelfLoop(Role),
    NotSelf(Role),
    /// A data range: the filler of a data role's `All`, `AtLeast` or `AtMost` (never
    /// `rdfs:Literal`, which is `Top`).
    Data(DataRangeExpr),
}

/// The complement of a data range (relative to the data domain).
fn complement(e: DataRangeExpr) -> DataRangeExpr {
    match e {
        DataRangeExpr::Not(x) => *x,
        e => DataRangeExpr::Not(Box::new(e)),
    }
}

fn negate(c: &Nnf) -> Nnf {
    match c {
        Nnf::Top => Nnf::Bottom,
        Nnf::Bottom => Nnf::Top,
        Nnf::Atom(a) => Nnf::Not(*a),
        Nnf::Not(a) => Nnf::Atom(*a),
        Nnf::And(cs) => Nnf::Or(cs.iter().map(negate).collect()),
        Nnf::Or(cs) => Nnf::And(cs.iter().map(negate).collect()),
        Nnf::All(r, f) => at_least(1, *r, negate(f)),
        Nnf::AtLeast(n, r, f) => at_most(n - 1, *r, (**f).clone()),
        Nnf::AtMost(n, r, f) => at_least(n + 1, *r, (**f).clone()),
        Nnf::SelfLoop(r) => Nnf::NotSelf(*r),
        Nnf::NotSelf(r) => Nnf::SelfLoop(*r),
        Nnf::Data(e) => Nnf::Data(complement(e.clone())),
    }
}

fn at_least(n: u32, r: Role, f: Nnf) -> Nnf {
    if n == 0 {
        Nnf::Top
    } else if f == Nnf::Bottom {
        Nnf::Bottom
    } else {
        Nnf::AtLeast(n, r, Box::new(f))
    }
}

fn at_most(n: u32, r: Role, f: Nnf) -> Nnf {
    if f == Nnf::Bottom {
        Nnf::Top
    } else if n == 0 {
        Nnf::All(r, Box::new(negate(&f)))
    } else {
        Nnf::AtMost(n, r, Box::new(f))
    }
}

struct Builder<'a> {
    v: Vocab,
    out: Clauses,
    class_atoms: FxHashMap<Id, Atom>,
    role_idx: FxHashMap<Id, u32>,
    inclusions: Vec<(Role, Role)>,
    /// Role inclusions `S₁ ∘ … ∘ Sₙ ⊑ R` with `n ≥ 2` (transitivity is `R ∘ R ⊑ R`), each
    /// also with its inverse `Sₙ⁻ ∘ … ∘ S₁⁻ ⊑ R⁻`, and their axiom.
    chains: Vec<(Vec<Role>, Role, usize)>,
    /// The roles with a chain below them (`w ⊑ S ⊑* R`, `|w| ≥ 2`).
    nonsimple: FxHashSet<Role>,
    /// The automaton of each non-simple role, by the least role equivalent to it.
    automata: FxHashMap<Role, Nfa>,
    individuals: FxHashMap<Individual, u32>,
    at_least_atoms: FxHashMap<(u32, Role, Option<Atom>), Atom>,
    define_pos: FxHashMap<Nnf, Atom>,
    define_neg: FxHashMap<Nnf, Atom>,
    chain_all: FxHashMap<(Role, Nnf), Atom>,
    /// GCIs `⊤ ⊑ C` waiting for clausification, with their axiom index.
    gcis: Vec<(Nnf, u32)>,
    /// The axiom whose clause would exceed [`MAX_CLAUSE_VARS`], if any.
    too_wide: Option<usize>,
    axiom: usize,
    dict: &'a Dict,
    data_atoms: FxHashMap<DataRangeExpr, Atom>,
    /// The distinct ranges of each `DatatypeDefinition`, by defined datatype.
    definitions: FxHashMap<Id, Vec<DataRange>>,
    /// Datatypes with two definitions of the same range, one of them annotated: whether
    /// the axioms are one depends on their annotations, which are not compared.
    annotated_repeats: FxHashSet<Id>,
    /// Work done expanding data ranges, against `max_work`.
    work: Cell<u64>,
    max_work: u64,
    /// The encoding of `owl:topObjectProperty`, once a restriction uses it: see
    /// [`Builder::universal`].
    universal: Option<(Role, Atom)>,
}

/// One of the two built-in object properties.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Extreme {
    /// `owl:topObjectProperty`, relating every pair of individuals.
    Top,
    /// `owl:bottomObjectProperty`, relating none.
    Bottom,
}

/// The largest cardinality in a number restriction the clausifier translates.
pub const MAX_CARDINALITY: u32 = 64;

/// The most states the automaton of one non-simple role may have.
pub const MAX_AUTOMATON_STATES: u32 = 4096;

/// The deepest nesting of non-simple roles one automaton may embed (each level is a
/// recursive construction step).
pub const MAX_AUTOMATON_DEPTH: usize = 256;

/// A finite automaton over roles: state 0 is initial and state 1 final.
#[derive(Clone, Debug)]
struct Nfa {
    states: u32,
    trans: Vec<(u32, Role, u32)>,
    eps: Vec<(u32, u32)>,
}

impl Nfa {
    /// Whether the automaton is past [`MAX_AUTOMATON_STATES`] (or its transitions past a
    /// small multiple of it).
    fn too_big(&self) -> bool {
        self.states > MAX_AUTOMATON_STATES
            || self.trans.len() + self.eps.len() > 4 * MAX_AUTOMATON_STATES as usize
    }

    /// Adds a copy of `sub` read between `from` and `to`.
    fn embed(&mut self, sub: &Nfa, from: u32, to: u32) {
        let o = self.states;
        self.states += sub.states;
        self.trans
            .extend(sub.trans.iter().map(|&(q, s, q2)| (q + o, s, q2 + o)));
        self.eps
            .extend(sub.eps.iter().map(|&(q, q2)| (q + o, q2 + o)));
        self.eps.push((from, o));
        self.eps.push((o + 1, to));
    }
}

/// The most successor variables one DL-clause may have.
const MAX_CLAUSE_VARS: u32 = 1024;

/// Translates `onto` to DL-clauses, or names the first axiom outside the fragment. Data
/// range expansion may do at most `max_work` units of work.
pub(crate) fn clausify(
    dict: &Dict,
    onto: &Ontology,
    max_work: u64,
) -> Result<Clauses, Unsupported> {
    let mut b = Builder {
        v: Vocab::intern(dict),
        out: Clauses::default(),
        class_atoms: FxHashMap::default(),
        role_idx: FxHashMap::default(),
        inclusions: Vec::new(),
        chains: Vec::new(),
        nonsimple: FxHashSet::default(),
        automata: FxHashMap::default(),
        individuals: FxHashMap::default(),
        at_least_atoms: FxHashMap::default(),
        define_pos: FxHashMap::default(),
        define_neg: FxHashMap::default(),
        chain_all: FxHashMap::default(),
        gcis: Vec::new(),
        too_wide: None,
        axiom: 0,
        dict,
        data_atoms: FxHashMap::default(),
        definitions: FxHashMap::default(),
        annotated_repeats: FxHashSet::default(),
        work: Cell::new(0),
        max_work,
        universal: None,
    };
    // Each datatype's distinct ranges, compared as the profile check compares them.
    let mut seen: FxHashMap<Id, Vec<(DataRange, bool)>> = FxHashMap::default();
    for (i, rec) in onto.axioms.iter().enumerate() {
        if let Axiom::DatatypeDefinition(dt, r) = &rec.axiom {
            let (range, annotated) = (canonical_range(r), onto.is_annotated(i));
            let reps = seen.entry(*dt).or_default();
            match reps.iter_mut().find(|(r, _)| *r == range) {
                Some((_, other)) => {
                    if annotated || *other {
                        b.annotated_repeats.insert(*dt);
                    }
                    *other |= annotated;
                }
                None => {
                    reps.push((range.clone(), annotated));
                    b.definitions.entry(*dt).or_default().push(range);
                }
            }
        }
    }
    // Role axioms first: the automata encoding of `∀` needs the whole hierarchy.
    for (i, rec) in onto.axioms.iter().enumerate() {
        b.axiom = i;
        b.role_axiom(&rec.axiom)?;
    }
    b.close_hierarchy();
    b.build_automata()?;
    for (i, rec) in onto.axioms.iter().enumerate() {
        b.axiom = i;
        b.class_axiom(&rec.axiom)?;
    }
    // Class axioms may mention properties no role axiom does: close over all of them.
    b.close_hierarchy();
    while let Some((c, src)) = b.gcis.pop() {
        b.clausify_gci(c, src);
        if let Some(axiom) = b.too_wide {
            return Err(Unsupported {
                construct: "an axiom whose clause needs more than MAX_CLAUSE_VARS successors",
                axiom,
                kind: Refusal::Fragment,
            });
        }
    }
    let mut classes: Vec<(Id, Atom)> = b.class_atoms.iter().map(|(&c, &a)| (c, a)).collect();
    classes.sort_by_key(|&(_, a)| a);
    b.out.classes = classes;
    Ok(b.out)
}

impl Builder<'_> {
    /// Number restrictions are decided up to [`MAX_CARDINALITY`]: an at-most restriction
    /// becomes a clause over `n + 1` successors and `n(n + 1)/2` equalities, and an at-least
    /// one creates `n` nodes.
    fn cardinality(&self, n: u32) -> Result<(), Unsupported> {
        if n > MAX_CARDINALITY {
            return Err(self.unsupported("a number restriction above MAX_CARDINALITY"));
        }
        Ok(())
    }

    fn unsupported(&self, construct: &'static str) -> Unsupported {
        Unsupported {
            construct,
            axiom: self.axiom,
            kind: Refusal::Fragment,
        }
    }

    fn undecidable_data(&self, construct: &'static str) -> Unsupported {
        Unsupported {
            construct,
            axiom: self.axiom,
            kind: Refusal::Datatype,
        }
    }

    fn fresh(&mut self, kind: AtomKind) -> Atom {
        let a = self.out.atoms.len() as Atom;
        self.out.atoms.push(kind);
        a
    }

    fn class(&mut self, id: Id) -> Atom {
        if let Some(&a) = self.class_atoms.get(&id) {
            return a;
        }
        let a = self.fresh(AtomKind::Class(id));
        self.class_atoms.insert(id, a);
        a
    }

    fn role(&mut self, p: &ObjectProperty) -> Result<Role, Unsupported> {
        let id = p.named();
        if id == self.v.top_object_property || id == self.v.bottom_object_property {
            return Err(self.unsupported("owl:topObjectProperty / owl:bottomObjectProperty"));
        }
        let idx = self.role_index(id, false)?;
        Ok(Role {
            idx,
            inv: p.is_inverse(),
        })
    }

    /// Which built-in object property `p` is (its inverse is itself), if any.
    fn extreme(&self, p: &ObjectProperty) -> Option<Extreme> {
        let id = p.named();
        if id == self.v.top_object_property {
            Some(Extreme::Top)
        } else if id == self.v.bottom_object_property {
            Some(Extreme::Bottom)
        } else {
            None
        }
    }

    /// `⊤ ⊑ ∀P.⊥`: `P` relates nothing. Only an empty domain leaves the top property
    /// empty, so that has no model.
    fn empty_role(&mut self, p: &ObjectProperty) -> Result<(), Unsupported> {
        match self.extreme(p) {
            Some(Extreme::Bottom) => {}
            Some(Extreme::Top) => self.gci(Nnf::Bottom),
            None => {
                let r = self.role(p)?;
                self.gci(Nnf::All(r, Box::new(Nnf::Bottom)));
            }
        }
        Ok(())
    }

    /// The encoding of `owl:topObjectProperty` `U`: a fresh role `W` (indexed by the top
    /// property itself) and nominal `{o}` with `⊤ ⊑ ∃W.{o}`. Every element reaches `o`,
    /// so `o`'s `W⁻`-successors are all elements, and a restriction `Q U.C` (`∃` or `∀`)
    /// holds exactly where `∃W.({o} ⊓ Q W⁻.C)` does.
    fn universal(&mut self) -> Result<(Role, Atom), Unsupported> {
        if let Some(u) = self.universal {
            return Ok(u);
        }
        let id = self.v.top_object_property;
        let w = Role {
            idx: self.role_index(id, false)?,
            inv: false,
        };
        // An anonymous individual: no key applies to it, and no IRI can name it.
        let o = self.nominal(&Individual::Anonymous(id));
        self.gci(at_least(1, w, Nnf::Atom(o)));
        self.universal = Some((w, o));
        Ok((w, o))
    }

    /// `Q U.C` through [`universal`](Self::universal), for `Q W⁻.C` given by `inner`.
    fn through_universal(&mut self, inner: impl FnOnce(Role) -> Nnf) -> Result<Nnf, Unsupported> {
        let (w, o) = self.universal()?;
        Ok(at_least(
            1,
            w,
            Nnf::And(vec![Nnf::Atom(o), inner(w.inverse())]),
        ))
    }

    /// The role of a data property.
    fn data_role(&mut self, id: Id) -> Result<Role, Unsupported> {
        if id == self.v.top_data_property || id == self.v.bottom_data_property {
            return Err(self.unsupported("owl:topDataProperty / owl:bottomDataProperty"));
        }
        let idx = self.role_index(id, true)?;
        Ok(Role { idx, inv: false })
    }

    fn role_index(&mut self, id: Id, data: bool) -> Result<u32, Unsupported> {
        match self.role_idx.get(&id) {
            Some(&i) if self.out.data_roles[i as usize] == data => Ok(i),
            Some(_) => {
                Err(self.unsupported("a property used as both an object and a data property"))
            }
            None => {
                let i = self.out.roles.len() as u32;
                self.out.roles.push(id);
                self.out.data_roles.push(data);
                self.role_idx.insert(id, i);
                Ok(i)
            }
        }
    }

    /// The value of a literal of a data range or assertion. Ill-typed literals make the
    /// ontology inconsistent and are handled before clausification; here they, values this
    /// implementation does not represent, datatypes outside the datatype map and
    /// `rdf:XMLLiteral` (whose XML content is not checked) are refused.
    fn value(&self, l: &Literal) -> Result<Value, Unsupported> {
        if l.datatype == format!("{RDF}XMLLiteral") {
            return Err(self.undecidable_data("an rdf:XMLLiteral literal"));
        }
        match datatypes::parse_value(&l.lexical, &l.datatype, l.lang.as_deref()) {
            Ok(v) => Ok(v),
            Err(ValueError::Unsupported) => {
                Err(self.undecidable_data("a literal value beyond the represented range"))
            }
            Err(ValueError::IllTyped) => Err(self.undecidable_data("an ill-typed facet value")),
            Err(ValueError::UnknownDatatype) => {
                Err(self.unsupported("a literal of a datatype outside the OWL 2 datatype map"))
            }
        }
    }

    /// `dr` as a [`DataRangeExpr`], with defined datatypes expanded. `expanding` holds the
    /// datatypes whose definitions are being expanded, so a cycle is refused.
    fn data_expr(
        &self,
        dr: &DataRange,
        expanding: &mut Vec<Id>,
    ) -> Result<DataRangeExpr, Unsupported> {
        /// Nested definitions deeper than this are refused (the oracle answers Unknown past
        /// the same depth anyway).
        const MAX_EXPANSION: usize = 64;
        let iri = |id: Id| iri_of(self.dict, id).unwrap_or_default();
        // Expansion can grow exponentially (each definition naming the previous one twice):
        // every node built counts against the budget.
        self.work.set(self.work.get() + 1);
        if self.work.get() > self.max_work {
            return Err(Unsupported {
                construct: "data ranges expanding past the work budget",
                axiom: self.axiom,
                kind: Refusal::Budget,
            });
        }
        Ok(match dr {
            DataRange::TopLiteral => DataRangeExpr::Datatype(Builtin::RdfsLiteral),
            DataRange::Datatype(id) => {
                if let Some(b) = builtin(&iri(*id)) {
                    return Ok(DataRangeExpr::Datatype(b));
                }
                let def = match self.definitions.get(id).map(Vec::as_slice) {
                    Some([def]) => def,
                    Some(_) => {
                        return Err(
                            self.unsupported("a datatype with more than one DatatypeDefinition")
                        )
                    }
                    None => return Err(self.unsupported(
                        "a datatype outside the OWL 2 datatype map without a DatatypeDefinition",
                    )),
                };
                if expanding.contains(id) {
                    return Err(self.unsupported("a cyclic DatatypeDefinition"));
                }
                if self.annotated_repeats.contains(id) {
                    return Err(self.undecidable_data(
                        "a DatatypeDefinition repeated with annotations (equality not compared)",
                    ));
                }
                if expanding.len() >= MAX_EXPANSION {
                    return Err(self.undecidable_data("DatatypeDefinitions nested too deeply"));
                }
                expanding.push(*id);
                let e = self.data_expr(def, expanding);
                expanding.pop();
                e?
            }
            DataRange::IntersectionOf(rs) => DataRangeExpr::And(
                rs.iter()
                    .map(|r| self.data_expr(r, expanding))
                    .collect::<Result<_, _>>()?,
            ),
            DataRange::UnionOf(rs) => DataRangeExpr::Or(
                rs.iter()
                    .map(|r| self.data_expr(r, expanding))
                    .collect::<Result<_, _>>()?,
            ),
            DataRange::ComplementOf(r) => {
                DataRangeExpr::Not(Box::new(self.data_expr(r, expanding)?))
            }
            DataRange::OneOf(ls) => {
                DataRangeExpr::OneOf(ls.iter().map(|l| self.value(l)).collect::<Result<_, _>>()?)
            }
            DataRange::Restriction(id, facets) => {
                let Some(b) = builtin(&iri(*id)) else {
                    return Err(self.unsupported(
                        "a facet restriction of a datatype outside the OWL 2 datatype map",
                    ));
                };
                let mut fs = Vec::with_capacity(facets.len());
                for (f, l) in facets {
                    let Some(f) = datatypes::facet(&iri(*f)) else {
                        return Err(self.unsupported("a facet outside OWL 2"));
                    };
                    fs.push((f, self.value(l)?));
                }
                DataRangeExpr::Restriction(b, fs)
            }
        })
    }

    /// A data range as the filler of a data restriction: `Top` for `rdfs:Literal`, otherwise
    /// [`Nnf::Data`]. A range the datatype oracle cannot decide is refused.
    fn data_nnf(&self, dr: &DataRange) -> Result<Nnf, Unsupported> {
        let e = self.data_expr(dr, &mut Vec::new())?;
        self.data_filler(e)
    }

    fn data_filler(&self, e: DataRangeExpr) -> Result<Nnf, Unsupported> {
        if e == DataRangeExpr::Datatype(Builtin::RdfsLiteral) {
            return Ok(Nnf::Top);
        }
        if data_ranges::is_empty(&e) == Tri::Unknown {
            return Err(self.undecidable_data("a data range the datatype oracle cannot decide"));
        }
        Ok(Nnf::Data(e))
    }

    /// The atom of a data range.
    fn data_atom(&mut self, e: DataRangeExpr) -> Atom {
        if let Some(&a) = self.data_atoms.get(&e) {
            return a;
        }
        let i = self.out.data.len() as u32;
        self.out.data.push(e.clone());
        self.out.data_source.push(self.src());
        let a = self.fresh(AtomKind::Data(i));
        self.data_atoms.insert(e, a);
        a
    }

    fn individual(&mut self, a: &Individual) -> u32 {
        if let Some(&i) = self.individuals.get(a) {
            return i;
        }
        let i = self.out.abox.individuals.len() as u32;
        self.out.abox.individuals.push(*a);
        self.individuals.insert(*a, i);
        i
    }

    /// The nominal atom `{a}`, asserted of `a` itself.
    fn nominal(&mut self, a: &Individual) -> Atom {
        let i = self.individual(a);
        if let Some(&o) = self.out.nominals.get(&i) {
            return o;
        }
        let o = self.fresh(AtomKind::Nominal(i));
        self.out.nominals.insert(i, o);
        self.out.abox.concepts.push((o, i, self.axiom as u32));
        o
    }

    fn include(&mut self, r: Role, s: Role) {
        self.inclusions.push((r, s));
        self.inclusions.push((r.inverse(), s.inverse()));
    }

    fn role_axiom(&mut self, ax: &Axiom) -> Result<(), Unsupported> {
        match ax {
            // Every property is below the top property; a chain through the bottom property
            // relates nothing.
            Axiom::SubObjectPropertyOf(_, sup) if self.extreme(sup) == Some(Extreme::Top) => {}
            Axiom::SubObjectPropertyOf(chain, _)
                if chain
                    .iter()
                    .any(|r| self.extreme(r) == Some(Extreme::Bottom)) => {}
            Axiom::SubObjectPropertyOf(chain, sup)
                if self.extreme(sup) == Some(Extreme::Bottom) =>
            {
                // No `R₁ ∘ … ∘ Rₙ`-path: `⊤ ⊑ ∀R₁.…∀Rₙ.⊥` (through the automata of
                // non-simple ones).
                let w = chain
                    .iter()
                    .map(|r| self.role(r))
                    .collect::<Result<Vec<_>, _>>()?;
                let f = w
                    .iter()
                    .rev()
                    .fold(Nnf::Bottom, |f, &r| Nnf::All(r, Box::new(f)));
                self.gci(f);
            }
            Axiom::SubObjectPropertyOf(chain, sup) => {
                let s = self.role(sup)?;
                let w = chain
                    .iter()
                    .map(|r| self.role(r))
                    .collect::<Result<Vec<_>, _>>()?;
                if let [r] = w.as_slice() {
                    self.include(*r, s);
                } else {
                    self.chain(w, s);
                }
            }
            Axiom::EquivalentObjectProperties(ps)
                if ps.iter().any(|p| self.extreme(p) == Some(Extreme::Bottom)) =>
            {
                for p in ps {
                    self.empty_role(p)?;
                }
            }
            Axiom::EquivalentObjectProperties(ps)
                if ps.iter().all(|p| self.extreme(p) == Some(Extreme::Top)) => {}
            Axiom::EquivalentObjectProperties(ps) => {
                let rs = ps
                    .iter()
                    .map(|p| self.role(p))
                    .collect::<Result<Vec<_>, _>>()?;
                for w in rs.windows(2) {
                    self.include(w[0], w[1]);
                    self.include(w[1], w[0]);
                }
            }
            Axiom::InverseObjectProperties(p, q) if self.extreme(p) == Some(Extreme::Bottom) => {
                self.empty_role(q)?;
            }
            Axiom::InverseObjectProperties(p, q) if self.extreme(q) == Some(Extreme::Bottom) => {
                self.empty_role(p)?;
            }
            Axiom::InverseObjectProperties(p, q)
                if self.extreme(p) == Some(Extreme::Top)
                    && self.extreme(q) == Some(Extreme::Top) => {}
            Axiom::InverseObjectProperties(p, q) => {
                let (r, s) = (self.role(p)?, self.role(q)?);
                self.include(r, s.inverse());
                self.include(s.inverse(), r);
            }
            // Both built-in properties are symmetric and transitive.
            Axiom::SymmetricObjectProperty(p) | Axiom::TransitiveObjectProperty(p)
                if self.extreme(p).is_some() => {}
            Axiom::SymmetricObjectProperty(p) => {
                let r = self.role(p)?;
                self.include(r, r.inverse());
            }
            Axiom::TransitiveObjectProperty(p) => {
                let r = self.role(p)?;
                self.chain(vec![r, r], r);
            }
            // Every data property is below owl:topDataProperty, and owl:bottomDataProperty
            // below every one: nothing to add.
            Axiom::SubDataPropertyOf(_, e) if *e == self.v.top_data_property => {}
            Axiom::SubDataPropertyOf(d, _) if *d == self.v.bottom_data_property => {}
            Axiom::SubDataPropertyOf(d, e) if *e == self.v.bottom_data_property => {
                let r = self.data_role(*d)?;
                self.gci(Nnf::All(r, Box::new(Nnf::Bottom)));
            }
            Axiom::EquivalentDataProperties(ds) if ds.contains(&self.v.bottom_data_property) => {
                let bottom = self.v.bottom_data_property;
                for &d in ds.iter().filter(|&&d| d != bottom) {
                    let r = self.data_role(d)?;
                    self.gci(Nnf::All(r, Box::new(Nnf::Bottom)));
                }
            }
            Axiom::SubDataPropertyOf(d, e) => {
                let (r, s) = (self.data_role(*d)?, self.data_role(*e)?);
                self.include(r, s);
            }
            Axiom::EquivalentDataProperties(ds) => {
                let rs = ds
                    .iter()
                    .map(|d| self.data_role(*d))
                    .collect::<Result<Vec<_>, _>>()?;
                for w in rs.windows(2) {
                    self.include(w[0], w[1]);
                    self.include(w[1], w[0]);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn close_hierarchy(&mut self) {
        let n = 2 * self.out.roles.len();
        let key = |r: Role| (2 * r.idx + u32::from(r.inv)) as usize;
        let mut direct: Vec<Vec<Role>> = vec![Vec::new(); n];
        for &(r, s) in &self.inclusions {
            direct[key(r)].push(s);
        }
        let mut supers = Vec::with_capacity(n);
        for k in 0..n {
            let start = Role {
                idx: (k / 2) as u32,
                inv: k % 2 == 1,
            };
            let mut seen = vec![start];
            let mut i = 0;
            while i < seen.len() {
                for &s in &direct[key(seen[i])] {
                    if !seen.contains(&s) {
                        seen.push(s);
                    }
                }
                i += 1;
            }
            supers.push(seen);
        }
        self.out.supers = supers;
    }

    /// Records `w ⊑ s` and its inverse.
    fn chain(&mut self, w: Vec<Role>, s: Role) {
        let inv = w.iter().rev().map(|r| r.inverse()).collect();
        self.chains.push((w, s, self.axiom));
        self.chains.push((inv, s.inverse(), self.axiom));
    }

    /// Whether `r` and `s` imply each other (`r ⊑* s ⊑* r`).
    fn equivalent(&self, r: Role, s: Role) -> bool {
        self.out.supers(r).contains(&s) && self.out.supers(s).contains(&r)
    }

    /// The least role equivalent to `r`.
    fn representative(&self, r: Role) -> Role {
        self.out
            .supers(r)
            .iter()
            .copied()
            .filter(|&s| self.equivalent(r, s))
            .min()
            .unwrap_or(r)
    }

    /// Marks the non-simple roles and builds their automata, on the hierarchy of the role
    /// axioms (a role first mentioned by a class axiom has nothing below it).
    fn build_automata(&mut self) -> Result<(), Unsupported> {
        for i in 0..self.chains.len() {
            let above = self.out.supers(self.chains[i].1).to_vec();
            self.nonsimple.extend(above);
        }
        let mut roles: Vec<Role> = self.nonsimple.iter().copied().collect();
        roles.sort();
        for r in roles {
            let k = self.representative(r);
            self.automaton(k, &mut Vec::new())?;
        }
        Ok(())
    }

    /// The automaton of the representative `k`: state 0 initial, state 1 final, accepting
    /// the role words `w` with `w ⊑* k`. The words of a non-simple role met on the way are
    /// those of its own automaton, embedded between ε-transitions; a simple role is one
    /// transition, since the tableau materialises the hierarchy. A hierarchy on which the
    /// construction does not terminate (not regular) or grows past [`MAX_AUTOMATON_STATES`]
    /// is refused.
    fn automaton(&mut self, k: Role, stack: &mut Vec<Role>) -> Result<Nfa, Unsupported> {
        if let Some(a) = self.automata.get(&k) {
            return Ok(a.clone());
        }
        let too_big = |b: &Self| {
            b.unsupported("a property hierarchy whose automaton exceeds MAX_AUTOMATON_STATES")
        };
        if stack.contains(&k) {
            return Err(self.unsupported("an irregular property hierarchy"));
        }
        if stack.len() > MAX_AUTOMATON_DEPTH {
            return Err(
                self.unsupported("a property hierarchy nested deeper than MAX_AUTOMATON_DEPTH")
            );
        }
        stack.push(k);
        let mut a = Nfa {
            states: 2,
            trans: vec![(0, k, 1)],
            eps: Vec::new(),
        };
        // `S ⊑ R` with a non-simple `S`: the words of `S`.
        let mut subs: Vec<Role> = self
            .inclusions
            .iter()
            .filter(|&&(s, t)| {
                self.equivalent(t, k) && !self.equivalent(s, k) && self.nonsimple.contains(&s)
            })
            .map(|&(s, _)| self.representative(s))
            .collect();
        subs.sort();
        subs.dedup();
        for s in subs {
            let sub = self.automaton(s, stack)?;
            a.embed(&sub, 0, 1);
            if a.too_big() {
                return Err(too_big(self));
            }
        }
        let chains: Vec<(Vec<Role>, usize)> = self
            .chains
            .iter()
            .filter(|(_, t, _)| self.equivalent(*t, k))
            .map(|(w, _, ax)| (w.clone(), *ax))
            .collect();
        for (w, ax) in chains {
            self.axiom = ax;
            let n = w.len();
            let (first, last) = (self.equivalent(w[0], k), self.equivalent(w[n - 1], k));
            let middle = &w[usize::from(first)..n - usize::from(last)];
            if middle.iter().any(|&s| self.equivalent(s, k)) {
                return Err(self.unsupported("an irregular property hierarchy"));
            }
            let (from, to) = match (first, last) {
                // `R ∘ R ⊑ R`.
                (true, true) if middle.is_empty() => {
                    a.eps.push((1, 0));
                    if a.too_big() {
                        return Err(too_big(self));
                    }
                    continue;
                }
                (true, true) => return Err(self.unsupported("an irregular property hierarchy")),
                // `R ∘ S₁ ∘ … ⊑ R` loops on the final state, `… ∘ Sₙ ∘ R ⊑ R` on the initial.
                (true, false) => (1, 1),
                (false, true) => (0, 0),
                (false, false) => (0, 1),
            };
            let mut cur = from;
            for (j, &s) in middle.iter().enumerate() {
                let next = if j + 1 == middle.len() {
                    to
                } else {
                    a.states += 1;
                    a.states - 1
                };
                if self.nonsimple.contains(&s) {
                    let sub = self.automaton(self.representative(s), stack)?;
                    a.embed(&sub, cur, next);
                } else {
                    a.trans.push((cur, s, next));
                }
                if a.too_big() {
                    return Err(too_big(self));
                }
                cur = next;
            }
        }
        stack.pop();
        if a.too_big() {
            return Err(too_big(self));
        }
        self.automata.insert(k, a.clone());
        Ok(a)
    }

    fn src(&self) -> u32 {
        self.axiom as u32
    }

    fn gci(&mut self, c: Nnf) {
        let src = self.src();
        self.gcis.push((c, src));
    }

    fn class_axiom(&mut self, ax: &Axiom) -> Result<(), Unsupported> {
        match ax {
            Axiom::Declaration(EntityKind::Class, id) => {
                if *id != self.v.thing && *id != self.v.nothing {
                    self.class(*id);
                }
            }
            Axiom::Declaration(..)
            | Axiom::AnnotationAssertion(..)
            | Axiom::SubAnnotationPropertyOf(..)
            | Axiom::AnnotationPropertyDomain(..)
            | Axiom::AnnotationPropertyRange(..) => {}
            Axiom::SubClassOf(c, d) => {
                if let (ClassExpression::Class(a), ClassExpression::Class(b)) = (c, d) {
                    self.out.told.insert((*a, *b));
                }
                let (c, d) = (self.nnf(c, false)?, self.nnf(d, true)?);
                self.gci(Nnf::Or(vec![c, d]));
            }
            Axiom::EquivalentClasses(cs) => {
                for (i, c) in cs.iter().enumerate() {
                    for d in &cs[i + 1..] {
                        if let (ClassExpression::Class(a), ClassExpression::Class(b)) = (c, d) {
                            self.out.told.insert((*a, *b));
                            self.out.told.insert((*b, *a));
                        }
                    }
                }
                for w in cs.windows(2) {
                    self.sub(&w[0], &w[1])?;
                    self.sub(&w[1], &w[0])?;
                }
            }
            Axiom::DisjointClasses(cs) => self.disjoint(cs)?,
            Axiom::DisjointUnion(a, cs) => {
                let a = ClassExpression::Class(*a);
                let union = ClassExpression::UnionOf(cs.clone());
                self.sub(&a, &union)?;
                self.sub(&union, &a)?;
                self.disjoint(cs)?;
            }
            // The top property's domain and range are every element; the bottom one's empty.
            Axiom::ObjectPropertyDomain(p, c) | Axiom::ObjectPropertyRange(p, c)
                if self.extreme(p) == Some(Extreme::Top) =>
            {
                let c = self.nnf(c, true)?;
                self.gci(c);
            }
            Axiom::ObjectPropertyDomain(p, _) | Axiom::ObjectPropertyRange(p, _)
                if self.extreme(p) == Some(Extreme::Bottom) => {}
            Axiom::ObjectPropertyDomain(p, c) => {
                let r = self.role(p)?;
                let c = self.nnf(c, true)?;
                self.gci(Nnf::All(r.inverse(), Box::new(c)));
            }
            Axiom::ObjectPropertyRange(p, c) => {
                let r = self.role(p)?;
                let c = self.nnf(c, true)?;
                self.gci(Nnf::All(r, Box::new(c)));
            }
            Axiom::FunctionalObjectProperty(p) => {
                let r = self.role(p)?;
                self.gci(at_most(1, r, Nnf::Top));
            }
            Axiom::InverseFunctionalObjectProperty(p) => {
                let r = self.role(p)?;
                self.gci(at_most(1, r.inverse(), Nnf::Top));
            }
            // Every element relates to itself by the top property, and none by the bottom one.
            Axiom::ReflexiveObjectProperty(p) if self.extreme(p) == Some(Extreme::Top) => {}
            Axiom::ReflexiveObjectProperty(p) if self.extreme(p) == Some(Extreme::Bottom) => {
                self.gci(Nnf::Bottom);
            }
            Axiom::ReflexiveObjectProperty(p) => {
                let r = self.role(p)?;
                self.gci(Nnf::SelfLoop(r));
            }
            Axiom::IrreflexiveObjectProperty(p) => {
                let r = self.role(p)?;
                self.gci(Nnf::NotSelf(r));
            }
            Axiom::AsymmetricObjectProperty(p) => {
                let r = self.role(p)?;
                self.push_clause(Clause {
                    body_roles: vec![(r, 0), (r.inverse(), 0)],
                    n_y: 1,
                    ..Clause::default()
                });
            }
            Axiom::DisjointObjectProperties(ps) => {
                let rs = ps
                    .iter()
                    .map(|p| self.role(p))
                    .collect::<Result<Vec<_>, _>>()?;
                for (i, &r) in rs.iter().enumerate() {
                    for &s in &rs[i + 1..] {
                        self.push_clause(Clause {
                            body_roles: vec![(r, 0), (s, 0)],
                            n_y: 1,
                            ..Clause::default()
                        });
                    }
                }
            }
            // Role axioms were read in the first pass.
            Axiom::SubObjectPropertyOf(..)
            | Axiom::EquivalentObjectProperties(_)
            | Axiom::InverseObjectProperties(..)
            | Axiom::SymmetricObjectProperty(_)
            | Axiom::TransitiveObjectProperty(_) => {}
            Axiom::SameIndividual(is) => {
                let ids: Vec<u32> = is.iter().map(|a| self.individual(a)).collect();
                for w in ids.windows(2) {
                    let src = self.src();
                    self.out.abox.same.push((w[0], w[1], src));
                }
            }
            Axiom::DifferentIndividuals(is) => {
                let ids: Vec<u32> = is.iter().map(|a| self.individual(a)).collect();
                for (i, &a) in ids.iter().enumerate() {
                    for &b in &ids[i + 1..] {
                        let src = self.src();
                        self.out.abox.different.push((a, b, src));
                    }
                }
            }
            Axiom::ClassAssertion(c, a) => {
                let a = self.individual(a);
                let c = self.nnf(c, true)?;
                let atom = match c {
                    Nnf::Top => return Ok(()),
                    Nnf::Atom(x) => x,
                    other => self.define(other),
                };
                let src = self.src();
                self.out.abox.concepts.push((atom, a, src));
            }
            // The top property relates every pair, the bottom one none.
            Axiom::ObjectPropertyAssertion(p, a, b)
            | Axiom::NegativeObjectPropertyAssertion(p, a, b)
                if self.extreme(p).is_some() =>
            {
                self.individual(a);
                self.individual(b);
                let positive = matches!(ax, Axiom::ObjectPropertyAssertion(..));
                if positive != (self.extreme(p) == Some(Extreme::Top)) {
                    self.gci(Nnf::Bottom);
                }
            }
            Axiom::ObjectPropertyAssertion(p, a, b) => {
                let r = self.role(p)?;
                let (a, b) = (self.individual(a), self.individual(b));
                let src = self.src();
                self.out.abox.roles.push((r, a, b, src));
            }
            Axiom::NegativeObjectPropertyAssertion(p, a, b) => {
                // `Mₐ ⊑ ∀R.¬M_b` with markers on `a` and `b` only (they follow merges). As a
                // universal it goes through `R`'s automaton, so an `R`-path made of
                // sub-property or chain edges is caught too.
                let r = self.role(p)?;
                let (a, b) = (self.individual(a), self.individual(b));
                let (ma, mb) = (self.fresh(AtomKind::Fresh), self.fresh(AtomKind::Fresh));
                let src = self.src();
                self.out.abox.concepts.push((ma, a, src));
                self.out.abox.concepts.push((mb, b, src));
                self.gci(Nnf::Or(vec![
                    Nnf::Not(ma),
                    Nnf::All(r, Box::new(Nnf::Not(mb))),
                ]));
            }
            // No individual has a value on a bottom property: the key identifies nothing.
            Axiom::HasKey(_, ps, ds)
                if ps.iter().any(|p| self.extreme(p) == Some(Extreme::Bottom))
                    || ds.contains(&self.v.bottom_data_property) => {}
            Axiom::HasKey(c, ps, ds) => {
                let roles = ps
                    .iter()
                    .map(|p| self.role(p))
                    .collect::<Result<Vec<_>, _>>()?;
                // A key on a non-simple role would have to see the values reached along
                // paths, which the tableau does not materialise.
                if roles.iter().any(|r| self.nonsimple.contains(r)) {
                    return Err(self.unsupported("HasKey on a non-simple object property"));
                }
                let data = ds
                    .iter()
                    .map(|&d| self.data_role(d))
                    .collect::<Result<Vec<_>, _>>()?;
                let class = match self.nnf(c, true)? {
                    Nnf::Top => None,
                    // No element is in the class: the key says nothing.
                    Nnf::Bottom => return Ok(()),
                    Nnf::Atom(a) => Some(a),
                    c => {
                        // The key applies exactly to the class's members: `Q ≡ C`.
                        let q = self.fresh(AtomKind::Fresh);
                        self.gci(Nnf::Or(vec![Nnf::Not(q), c.clone()]));
                        self.gci(Nnf::Or(vec![negate(&c), Nnf::Atom(q)]));
                        Some(q)
                    }
                };
                self.out.keys.push(Key { class, roles, data });
            }
            // Read in the first pass.
            Axiom::SubDataPropertyOf(..) | Axiom::EquivalentDataProperties(_) => {}
            Axiom::DatatypeDefinition(dt, _) => {
                // A definition constrains the models even when the datatype is unused: a
                // built-in redefined, two different definitions or a cyclic one may have no
                // model. Expanding it refuses the latter two.
                if builtin(&iri_of(self.dict, *dt).unwrap_or_default()).is_some() {
                    return Err(self.unsupported("a DatatypeDefinition of a datatype-map datatype"));
                }
                self.data_expr(&DataRange::Datatype(*dt), &mut Vec::new())?;
            }
            Axiom::DisjointDataProperties(ds) => {
                // The bottom data property is disjoint from every one.
                let bottom = self.v.bottom_data_property;
                let rs = ds
                    .iter()
                    .filter(|&&d| d != bottom)
                    .map(|d| self.data_role(*d))
                    .collect::<Result<Vec<_>, _>>()?;
                for (i, &r) in rs.iter().enumerate() {
                    for &s in &rs[i + 1..] {
                        self.push_clause(Clause {
                            body_roles: vec![(r, 0), (s, 1)],
                            n_y: 2,
                            head: vec![Head::Ineq(Var::Y(0), Var::Y(1))],
                            ..Clause::default()
                        });
                    }
                }
            }
            // The bottom data property relates nothing.
            Axiom::DataPropertyDomain(d, _)
            | Axiom::DataPropertyRange(d, _)
            | Axiom::FunctionalDataProperty(d)
                if *d == self.v.bottom_data_property => {}
            Axiom::DataPropertyAssertion(d, a, _)
            | Axiom::NegativeDataPropertyAssertion(d, a, _)
                if *d == self.v.bottom_data_property =>
            {
                self.individual(a);
                if matches!(ax, Axiom::DataPropertyAssertion(..)) {
                    self.gci(Nnf::Bottom);
                }
            }
            Axiom::DataPropertyDomain(d, c) => {
                // `∃D.⊤ ⊑ C`: `D(x, y) → C(x)`.
                let r = self.data_role(*d)?;
                let c = self.nnf(c, true)?;
                self.gci(Nnf::Or(vec![Nnf::All(r, Box::new(Nnf::Bottom)), c]));
            }
            Axiom::DataPropertyRange(d, dr) => {
                let r = self.data_role(*d)?;
                let f = self.data_nnf(dr)?;
                self.gci(Nnf::All(r, Box::new(f)));
            }
            Axiom::FunctionalDataProperty(d) => {
                let r = self.data_role(*d)?;
                self.gci(at_most(1, r, Nnf::Top));
            }
            Axiom::DataPropertyAssertion(d, a, l) => {
                let r = self.data_role(*d)?;
                let a = self.individual(a);
                let v = self.value(l)?;
                let one = DataRangeExpr::OneOf(vec![v.clone()]);
                if data_ranges::is_empty(&one) == Tri::Unknown {
                    return Err(
                        self.undecidable_data("a literal the datatype oracle cannot decide")
                    );
                }
                let atom = self.data_atom(one);
                let src = self.src();
                self.out.abox.data.push((r, a, atom, v, src));
            }
            Axiom::NegativeDataPropertyAssertion(d, a, l) => {
                // `Mₐ ⊑ ∀D.¬{v}`, with the marker on `a` only.
                let r = self.data_role(*d)?;
                let a = self.individual(a);
                let v = self.value(l)?;
                let excluded = self.data_filler(complement(DataRangeExpr::OneOf(vec![v])))?;
                let ma = self.fresh(AtomKind::Fresh);
                let src = self.src();
                self.out.abox.concepts.push((ma, a, src));
                self.gci(Nnf::Or(vec![Nnf::Not(ma), Nnf::All(r, Box::new(excluded))]));
            }
        }
        Ok(())
    }

    fn sub(&mut self, c: &ClassExpression, d: &ClassExpression) -> Result<(), Unsupported> {
        let (c, d) = (self.nnf(c, false)?, self.nnf(d, true)?);
        self.gci(Nnf::Or(vec![c, d]));
        Ok(())
    }

    fn disjoint(&mut self, cs: &[ClassExpression]) -> Result<(), Unsupported> {
        let ns = cs
            .iter()
            .map(|c| self.nnf(c, false))
            .collect::<Result<Vec<_>, _>>()?;
        for (i, c) in ns.iter().enumerate() {
            for d in &ns[i + 1..] {
                self.gci(Nnf::Or(vec![c.clone(), d.clone()]));
            }
        }
        Ok(())
    }

    /// The NNF of `c` (`positive`) or of `¬c`.
    fn nnf(&mut self, c: &ClassExpression, positive: bool) -> Result<Nnf, Unsupported> {
        use ClassExpression as C;
        let n = match c {
            C::Class(id) => {
                if *id == self.v.thing {
                    Nnf::Top
                } else if *id == self.v.nothing {
                    Nnf::Bottom
                } else {
                    Nnf::Atom(self.class(*id))
                }
            }
            C::Thing => Nnf::Top,
            C::Nothing => Nnf::Bottom,
            C::IntersectionOf(cs) => Nnf::And(
                cs.iter()
                    .map(|c| self.nnf(c, true))
                    .collect::<Result<_, _>>()?,
            ),
            C::UnionOf(cs) => Nnf::Or(
                cs.iter()
                    .map(|c| self.nnf(c, true))
                    .collect::<Result<_, _>>()?,
            ),
            C::ComplementOf(c) => return self.nnf(c, !positive),
            C::SomeValuesFrom(p, _) | C::HasValue(p, _)
                if self.extreme(p) == Some(Extreme::Bottom) =>
            {
                Nnf::Bottom
            }
            C::AllValuesFrom(p, _) if self.extreme(p) == Some(Extreme::Bottom) => Nnf::Top,
            // Every individual is a value of the top property.
            C::HasValue(p, _) if self.extreme(p) == Some(Extreme::Top) => Nnf::Top,
            C::SomeValuesFrom(p, f) if self.extreme(p) == Some(Extreme::Top) => {
                let f = self.nnf(f, true)?;
                self.through_universal(|w| at_least(1, w, f))?
            }
            C::AllValuesFrom(p, f) if self.extreme(p) == Some(Extreme::Top) => {
                let f = self.nnf(f, true)?;
                self.through_universal(|w| Nnf::All(w, Box::new(f)))?
            }
            C::SomeValuesFrom(p, f) => {
                let (r, f) = (self.role(p)?, self.nnf(f, true)?);
                at_least(1, r, f)
            }
            C::AllValuesFrom(p, f) => {
                let (r, f) = (self.role(p)?, self.nnf(f, true)?);
                Nnf::All(r, Box::new(f))
            }
            C::HasSelf(p) => Nnf::SelfLoop(self.role(p)?),
            C::MinCardinality(n, p, f) => {
                self.cardinality(*n)?;
                let (r, f) = (self.role(p)?, self.nnf(f, true)?);
                at_least(*n, r, f)
            }
            C::MaxCardinality(n, p, f) => {
                self.cardinality(*n)?;
                let (r, f) = (self.role(p)?, self.nnf(f, true)?);
                at_most(*n, r, f)
            }
            C::ExactCardinality(n, p, f) => {
                self.cardinality(*n)?;
                let (r, f) = (self.role(p)?, self.nnf(f, true)?);
                Nnf::And(vec![at_least(*n, r, f.clone()), at_most(*n, r, f)])
            }
            C::OneOf(is) => Nnf::Or(is.iter().map(|a| Nnf::Atom(self.nominal(a))).collect()),
            C::HasValue(p, a) => {
                let r = self.role(p)?;
                at_least(1, r, Nnf::Atom(self.nominal(a)))
            }
            // The bottom data property has no values.
            C::DataSomeValuesFrom(d, _) | C::DataHasValue(d, _)
                if *d == self.v.bottom_data_property =>
            {
                Nnf::Bottom
            }
            C::DataAllValuesFrom(d, _) | C::DataMaxCardinality(_, d, _)
                if *d == self.v.bottom_data_property =>
            {
                Nnf::Top
            }
            C::DataMinCardinality(n, d, _) | C::DataExactCardinality(n, d, _)
                if *d == self.v.bottom_data_property =>
            {
                if *n == 0 {
                    Nnf::Top
                } else {
                    Nnf::Bottom
                }
            }
            C::DataSomeValuesFrom(d, dr) => {
                let (r, f) = (self.data_role(*d)?, self.data_nnf(dr)?);
                at_least(1, r, f)
            }
            C::DataAllValuesFrom(d, dr) => {
                let (r, f) = (self.data_role(*d)?, self.data_nnf(dr)?);
                Nnf::All(r, Box::new(f))
            }
            C::DataHasValue(d, l) => {
                let r = self.data_role(*d)?;
                let f = self.data_filler(DataRangeExpr::OneOf(vec![self.value(l)?]))?;
                at_least(1, r, f)
            }
            C::DataMinCardinality(n, d, dr) => {
                self.cardinality(*n)?;
                let (r, f) = (self.data_role(*d)?, self.data_nnf(dr)?);
                at_least(*n, r, f)
            }
            C::DataMaxCardinality(n, d, dr) => {
                self.cardinality(*n)?;
                let (r, f) = (self.data_role(*d)?, self.data_nnf(dr)?);
                at_most(*n, r, f)
            }
            C::DataExactCardinality(n, d, dr) => {
                self.cardinality(*n)?;
                let (r, f) = (self.data_role(*d)?, self.data_nnf(dr)?);
                Nnf::And(vec![at_least(*n, r, f.clone()), at_most(*n, r, f)])
            }
        };
        Ok(if positive { n } else { negate(&n) })
    }

    /// A fresh `Q` with `Q ⊑ c`.
    fn define(&mut self, c: Nnf) -> Atom {
        if let Some(&q) = self.define_pos.get(&c) {
            return q;
        }
        let q = self.fresh(AtomKind::Fresh);
        self.define_pos.insert(c.clone(), q);
        match c {
            Nnf::And(cs) => {
                for ci in cs {
                    self.gci(Nnf::Or(vec![Nnf::Not(q), ci]));
                }
            }
            c => self.gci(Nnf::Or(vec![Nnf::Not(q), c])),
        }
        q
    }

    /// A fresh `Q` with `c ⊑ Q`.
    fn define_superset(&mut self, c: Nnf) -> Atom {
        if let Some(&q) = self.define_neg.get(&c) {
            return q;
        }
        let q = self.fresh(AtomKind::Fresh);
        self.define_neg.insert(c.clone(), q);
        self.gci(Nnf::Or(vec![negate(&c), Nnf::Atom(q)]));
        q
    }

    fn at_least_atom(&mut self, n: u32, r: Role, f: Option<Atom>) -> Atom {
        if let Some(&a) = self.at_least_atoms.get(&(n, r, f)) {
            return a;
        }
        let a = self.fresh(AtomKind::AtLeast {
            n,
            role: r,
            filler: f,
        });
        self.at_least_atoms.insert((n, r, f), a);
        a
    }

    fn push_clause(&mut self, mut c: Clause) {
        c.source = self.src();
        // Branching tries the disjuncts in order: those that create no successors first.
        let atoms = &self.out.atoms;
        c.head.sort_by_key(|h| match h {
            Head::Concept(a, _) => matches!(atoms[*a as usize], AtomKind::AtLeast { .. }),
            _ => false,
        });
        self.out.clauses.push(c);
    }

    fn clausify_gci(&mut self, c: Nnf, src: u32) {
        self.axiom = src as usize;
        let mut ds = Vec::new();
        flatten_or(c, &mut ds);
        let mut cl = Clause::default();
        for d in ds {
            match d {
                Nnf::Top => return,
                Nnf::Bottom => {}
                Nnf::Atom(a) => cl.head.push(Head::Concept(a, Var::X)),
                Nnf::Not(a) => cl.body_x.push(a),
                Nnf::Or(_) => unreachable!("flattened"),
                d @ Nnf::And(_) => {
                    let q = self.define(d);
                    cl.head.push(Head::Concept(q, Var::X));
                }
                Nnf::AtLeast(n, r, f) => {
                    let filler = match *f {
                        Nnf::Top => None,
                        Nnf::Bottom => continue,
                        Nnf::Atom(a) => Some(a),
                        Nnf::Data(e) => Some(self.data_atom(e)),
                        other => Some(self.define(other)),
                    };
                    let a = self.at_least_atom(n, r, filler);
                    cl.head.push(Head::Concept(a, Var::X));
                }
                Nnf::All(r, f) => {
                    if !self.nonsimple.contains(&r) {
                        if !self.all_into(&mut cl, r, *f) {
                            return;
                        }
                    } else {
                        let p = self.chain_all(r, *f);
                        cl.head.push(Head::Concept(p, Var::X));
                    }
                }
                Nnf::AtMost(n, r, f) if self.out.is_data(r) => {
                    // `D(x, y₀) ∧ … ∧ D(x, yₙ) → ⋁ ¬R(yᵢ) ∨ ⋁ yᵢ ≈ yⱼ`.
                    let outside = match *f {
                        Nnf::Top => None,
                        Nnf::Bottom => return,
                        Nnf::Data(e) => Some(self.data_atom(complement(e))),
                        _ => unreachable!("a data role's filler is a data range"),
                    };
                    if u32::from(cl.n_y) + n + 1 > MAX_CLAUSE_VARS {
                        self.too_wide = Some(self.axiom);
                        return;
                    }
                    let base = cl.n_y;
                    for i in 0..=n as u16 {
                        cl.body_roles.push((r, base + i));
                        if let Some(a) = outside {
                            cl.head.push(Head::Concept(a, Var::Y(base + i)));
                        }
                    }
                    cl.ordered.extend((1..=n as u16).map(|i| base + i));
                    for i in 0..=n as u16 {
                        for j in i + 1..=n as u16 {
                            cl.head.push(Head::Eq(Var::Y(base + i), Var::Y(base + j)));
                        }
                    }
                    cl.n_y += n as u16 + 1;
                }
                Nnf::AtMost(n, r, f) => {
                    let filler = match *f {
                        Nnf::Top => None,
                        Nnf::Bottom => return,
                        Nnf::Atom(a) => Some(a),
                        other => Some(self.define_superset(other)),
                    };
                    if u32::from(cl.n_y) + n + 1 > MAX_CLAUSE_VARS {
                        self.too_wide = Some(self.axiom);
                        return;
                    }
                    let base = cl.n_y;
                    for i in 0..=n as u16 {
                        cl.body_roles.push((r, base + i));
                        if let Some(a) = filler {
                            cl.body_y.push((a, base + i));
                        }
                    }
                    cl.ordered.extend((1..=n as u16).map(|i| base + i));
                    for i in 0..=n as u16 {
                        for j in i + 1..=n as u16 {
                            cl.head.push(Head::Eq(Var::Y(base + i), Var::Y(base + j)));
                        }
                    }
                    cl.n_y += n as u16 + 1;
                }
                Nnf::SelfLoop(r) => cl.head.push(Head::SelfLoop(r)),
                Nnf::NotSelf(r) => cl.body_self.push(r),
                Nnf::Data(_) => unreachable!("a data range is only a data role's filler"),
            }
        }
        self.push_clause(cl);
    }

    /// Adds `∀r.f` as a disjunct of `cl` (body `r(x, y)`, head or body `f(y)`); `false` when
    /// the disjunct is `⊤` and the clause is a tautology.
    fn all_into(&mut self, cl: &mut Clause, r: Role, f: Nnf) -> bool {
        if u32::from(cl.n_y) + 1 > MAX_CLAUSE_VARS {
            // The caller drops the clause; `clausify` then refuses the axiom.
            self.too_wide = Some(self.axiom);
            return false;
        }
        let y = cl.n_y;
        match f {
            Nnf::Top => return false,
            Nnf::Bottom => {}
            Nnf::Atom(a) => cl.head.push(Head::Concept(a, Var::Y(y))),
            Nnf::Not(a) => cl.body_y.push((a, y)),
            Nnf::Data(e) => {
                let a = self.data_atom(e);
                cl.head.push(Head::Concept(a, Var::Y(y)));
            }
            other => {
                let q = self.define(other);
                cl.head.push(Head::Concept(q, Var::Y(y)));
            }
        }
        cl.body_roles.push((r, y));
        cl.n_y += 1;
        true
    }

    /// A fresh `P ⊑ ∀r.f` for a non-simple `r`: `P` is the initial state's concept of `r`'s
    /// automaton run over the graph, and the final state's concept implies `f`.
    fn chain_all(&mut self, r: Role, f: Nnf) -> Atom {
        let k = self.representative(r);
        if let Some(&p) = self.chain_all.get(&(k, f.clone())) {
            return p;
        }
        let a = self.automata[&k].clone();
        let x: Vec<Atom> = (0..a.states).map(|_| self.fresh(AtomKind::Fresh)).collect();
        self.chain_all.insert((k, f.clone()), x[0]);
        for &(q, s, q2) in &a.trans {
            self.push_clause(Clause {
                body_x: vec![x[q as usize]],
                body_roles: vec![(s, 0)],
                head: vec![Head::Concept(x[q2 as usize], Var::Y(0))],
                n_y: 1,
                ..Clause::default()
            });
        }
        for &(q, q2) in &a.eps {
            self.push_clause(Clause {
                body_x: vec![x[q as usize]],
                head: vec![Head::Concept(x[q2 as usize], Var::X)],
                ..Clause::default()
            });
        }
        self.gci(Nnf::Or(vec![Nnf::Not(x[1]), f]));
        x[0]
    }
}

fn flatten_or(c: Nnf, out: &mut Vec<Nnf>) {
    match c {
        Nnf::Or(cs) => {
            for c in cs {
                flatten_or(c, out);
            }
        }
        Nnf::And(cs) if cs.len() == 1 => flatten_or(cs.into_iter().next().unwrap(), out),
        Nnf::And(cs) if cs.is_empty() => out.push(Nnf::Top),
        c => out.push(c),
    }
}
