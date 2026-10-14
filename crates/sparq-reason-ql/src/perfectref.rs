// sq-t5bne (epic sq-pbz04): PerfectRef — the OWL 2 QL (DL-Lite_R) query-rewriting
// core (Calvanese, De Giacomo, Lembo, Lenzerini, Rosati, JAR 39(3) 2007).
// sq-pbz04.3.1: B2 literal-object atoms — `Term::Literal` variant added.
// A literal is treated exactly like `Const` for the applicability condition: it is never
// `_`-eligible (a literal constant is never an unbound position), never unifiable with a
// generated witness, and two distinct literals are never equal. Role inclusions carry the
// literal constant unchanged (the same constant-propagation that already works for IRI Const).
//
// Given a conjunctive query q and a DL-Lite_R TBox T, PerfectRef saturates a SET of CQs (a
// union of conjunctive queries, UCQ) to a fixpoint under two operations, so that evaluating the
// UCQ over the UNMODIFIED data returns exactly the CERTAIN answers of q under T:
//
//   (a) REWRITE — apply a positive inclusion of T BACKWARD to one atom of a CQ:
//         A1 ⊑ A2   and atom A2(x)          ⇒  A1(x)
//         ∃R  ⊑ A   and atom A(x)           ⇒  R(x, _)
//         ∃R⁻ ⊑ A   and atom A(x)           ⇒  R(_, x)
//         A  ⊑ ∃R   and atom R(x, _)        ⇒  A(x)        (object MUST be unbound — see below)
//         A  ⊑ ∃R⁻  and atom R(_, x)        ⇒  A(x)        (subject MUST be unbound)
//         R1 ⊑ R2   and atom R2(x,y)        ⇒  R1(x,y)     (with inverse direction handling)
//   (b) REDUCE — unify two atoms of a CQ (most-general unifier), then ANONYMISE: a variable that
//       becomes non-shared and non-distinguished turns into `_`, which can RE-ENABLE rewrites.
//
// THE LOAD-BEARING APPLICABILITY CONDITION (the #1 unsoundness trap, and the reason the
// CQ-shape gate exists): an existential-INTRODUCING inclusion (`A ⊑ ∃R`, `∃R ⊑ A`, …) may be
// applied to an atom ONLY when the variable in the existential-bound position is `_` — i.e. an
// UNBOUND, NON-DISTINGUISHED, NON-SHARED variable. Applying it to a bound/shared/answer variable
// would unsoundly drop the join the variable participates in. This module enforces that
// condition explicitly in [`applicable_to_role`] / the class-atom rewrite, and tracks unbound
// positions precisely. Mis-gating here SILENTLY loses or invents answers, so the logic is kept
// small, total, and oracle-tested against a hand-checked DL-Lite reference.

use crate::dllite::{Basic, Role, TBox};
use rustc_hash::FxHashSet;

/// A term in a rewritten CQ atom.
///
/// sq-pbz04.3.1: `Lit` variant added for B2 (literal-object role atoms).
/// A `Lit` is treated exactly like `Const` for the applicability condition — it is never
/// `_`-eligible and is rigid in the MGU (two distinct literals do not unify, a literal cannot
/// bind to a non-literal). Role inclusions carry the literal unchanged.
///
/// The literal's lexical form, datatype IRI, and language tag (if any) are stored as plain
/// `String`s so the `Term` enum can derive `Eq`/`Hash`/`Ord`. This is correct for the
/// rewriter's purpose: syntactic identity governs CQ-canonicalisation and the unifier; the
/// emitter reconstructs the original `oxrdf::Literal` from these fields at emit time.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Term {
    /// A named (distinguished or shared) variable — identity is its name.
    Var(String),
    /// A constant IRI (a bound answer constant from the original query).
    Const(String),
    /// An UNBOUND `_` — a fresh existential that occurs in exactly one position of the CQ. Each
    /// carries a unique id so two `_`s are never accidentally identified. This is the symbol the
    /// existential applicability condition turns on.
    Unbound(u32),
    /// An RDF literal constant in a role-atom object position (B2, sq-pbz04.3.1). Stored as
    /// `(lexical_value, datatype_iri, Option<language_tag>)` — all plain `String` so `Term`
    /// derives `Eq`/`Hash`/`Ord`. Treated as a rigid constant — never `_`-eligible, never
    /// unifiable with a generated witness, distinct literals never equal.
    Lit(String, String, Option<String>),
}

impl Term {
    /// Whether this term is an unbound `_` existential (the position an existential-introducing
    /// inclusion may consume). Used by the oracle tests to assert on rewrite output.
    #[cfg(test)]
    pub fn is_unbound(&self) -> bool {
        matches!(self, Term::Unbound(_))
    }
}

/// A rewritten atom: either a class atom `A(t)` or a role atom `R(t1, t2)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Atom {
    Class { class: String, arg: Term },
    Role { role: Role, s: Term, o: Term },
}

/// A conjunctive query in the rewriter's working form: a set of body atoms plus the set of
/// distinguished (answer) variable NAMES. Stored as a sorted Vec so two structurally-equal CQs
/// hash/compare equal regardless of construction order (the fixpoint dedup relies on this).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cq {
    pub atoms: Vec<Atom>,
    pub answer: Vec<String>,
}

impl Cq {
    /// Canonicalise a CQ: sort + dedup the atoms so two structurally-equal CQs (built in any
    /// order) hash/compare equal — the fixpoint dedup and the tree-witness folding both rely on
    /// this. `pub(crate)` so [`crate::treewitness`] can build canonical folded disjuncts without
    /// duplicating the normalisation logic (which would risk drift).
    pub(crate) fn normalised(mut atoms: Vec<Atom>, answer: Vec<String>) -> Cq {
        atoms.sort();
        atoms.dedup();
        Cq { atoms, answer }
    }
}

/// A monotone counter handing out fresh `_` ids so every introduced existential is distinct.
struct Fresh(u32);
impl Fresh {
    fn next(&mut self) -> Term {
        let t = Term::Unbound(self.0);
        self.0 += 1;
        t
    }
}

/// Whether a variable NAME is "bound" in `cq`: it is distinguished (an answer var) OR it is
/// shared (occurs in ≥2 atom positions). The existential applicability condition forbids firing
/// on a bound variable; an `_` (Unbound) is never bound by definition.
fn is_bound_var(cq: &Cq, name: &str) -> bool {
    if cq.answer.iter().any(|a| a == name) {
        return true;
    }
    let mut count = 0usize;
    for a in &cq.atoms {
        match a {
            Atom::Class { arg, .. } => {
                if matches!(arg, Term::Var(v) if v == name) {
                    count += 1;
                }
            }
            Atom::Role { s, o, .. } => {
                if matches!(s, Term::Var(v) if v == name) {
                    count += 1;
                }
                if matches!(o, Term::Var(v) if v == name) {
                    count += 1;
                }
            }
        }
        if count >= 2 {
            return true;
        }
    }
    false
}

/// A term position is "_-eligible" (may be consumed by an existential-introducing inclusion) iff
/// it is an `Unbound`, OR a `Var` that is neither distinguished nor shared (so it is morally a
/// `_`). Constants and literals are never eligible. sq-pbz04.3.1: `Literal` added.
fn underscore_eligible(cq: &Cq, t: &Term) -> bool {
    match t {
        Term::Unbound(_) => true,
        Term::Var(name) => !is_bound_var(cq, name),
        Term::Const(_) => false,
        // B2: a literal constant is never an unbound position — the applicability condition
        // cannot fire on it (no existential witness can be introduced at a ground literal).
        Term::Lit(..) => false,
    }
}

/// PerfectRef: rewrite the conjunctive query `q` (given as body atoms + answer variable names)
/// under TBox `T` into the UCQ of certain-answer-preserving CQs. Returns the set of CQs (the
/// input query is always included). Deterministic ordering is imposed at emission, not here.
pub fn perfect_ref(atoms: Vec<Atom>, answer: Vec<String>, tbox: &TBox) -> Vec<Cq> {
    let mut fresh = Fresh(seed_fresh(&atoms));
    let start = Cq::normalised(atoms, answer);
    let mut seen: FxHashSet<Cq> = FxHashSet::default();
    let mut worklist: Vec<Cq> = vec![start.clone()];
    seen.insert(start);

    while let Some(cq) = worklist.pop() {
        // (a) REWRITE every atom by every applicable positive inclusion.
        for (i, atom) in cq.atoms.iter().enumerate() {
            for rewritten in rewrite_atom(&cq, atom, tbox, &mut fresh) {
                let mut new_atoms = cq.atoms.clone();
                new_atoms[i] = rewritten;
                push_cq(Cq::normalised(new_atoms, cq.answer.clone()), &mut seen, &mut worklist);
            }
        }
        // (b) REDUCE: unify each unordered pair of atoms; anonymise; re-add.
        let n = cq.atoms.len();
        for a in 0..n {
            for b in (a + 1)..n {
                if let Some(reduced) = reduce(&cq, a, b) {
                    push_cq(reduced, &mut seen, &mut worklist);
                }
            }
        }
    }

    seen.into_iter().collect()
}

fn push_cq(cq: Cq, seen: &mut FxHashSet<Cq>, worklist: &mut Vec<Cq>) {
    if seen.insert(cq.clone()) {
        worklist.push(cq);
    }
}

/// Seed the fresh-id counter above any `Unbound` already present (there are none in a parsed
/// query, but keep the API total). `Literal` terms carry no id. sq-pbz04.3.1.
fn seed_fresh(atoms: &[Atom]) -> u32 {
    let mut max = 0u32;
    let mut bump = |t: &Term| {
        if let Term::Unbound(id) = t {
            max = max.max(id + 1);
        }
    };
    for a in atoms {
        match a {
            Atom::Class { arg, .. } => bump(arg),
            Atom::Role { s, o, .. } => {
                bump(s);
                bump(o);
            }
        }
    }
    max
}

/// All single-atom rewrites of `atom` in `cq` under every applicable positive inclusion of T.
fn rewrite_atom(cq: &Cq, atom: &Atom, tbox: &TBox, fresh: &mut Fresh) -> Vec<Atom> {
    let mut out = Vec::new();
    match atom {
        Atom::Class { class, arg } => {
            for incl in &tbox.concept_incl {
                if &incl.sup != class {
                    continue;
                }
                // B ⊑ A, atom A(arg) ⇒ depends on the shape of B.
                match &incl.sub {
                    Basic::Class(b) => {
                        // A1 ⊑ A2: always applicable (no existential introduced).
                        out.push(Atom::Class {
                            class: b.clone(),
                            arg: arg.clone(),
                        });
                    }
                    Basic::Exists(role) => {
                        // ∃R ⊑ A: A(x) ⇒ R(x, _). Introduces an existential in the OBJECT
                        // position — always sound (the `_` is brand new and unshared); the
                        // applicability restriction is on CONSUMING an existential, not on
                        // INTRODUCING one. (Calvanese et al.: the gr() function.)
                        let under = fresh.next();
                        out.push(role_atom(role, arg.clone(), under));
                    }
                }
            }
        }
        Atom::Role { role, s, o } => {
            // Existential-GENERATING super-inclusions: B ⊑ ∃R, atom R(x, _) ⇒ B(x) (object
            // unbound), or via inverse B ⊑ ∃R⁻, atom R(_, x) ⇒ B(x) (subject unbound). The
            // APPLICABILITY CONDITION lives here: the existential-bound position must be `_`.
            for (sub_b, gen_role) in &tbox.exists_super {
                // R(x,y) is generated by `B ⊑ ∃gen_role`. Match the atom's role against gen_role
                // (and gen_role's inverse), and require the FILLER position to be `_`-eligible.
                if let Some(named_arg) = applicable_exists_super(cq, role, s, o, gen_role) {
                    out.push(class_or_role_basic(sub_b, named_arg, fresh));
                }
            }
            // Role inclusions R1 ⊑ R2: atom R2(x,y) ⇒ R1(x,y), honouring inverse direction.
            for (r1, r2) in &tbox.role_incl {
                if let Some(sub_atom) = apply_role_inclusion(role, s, o, r1, r2) {
                    out.push(sub_atom);
                }
            }
        }
    }
    out
}

/// Build the body atom for `R(s, o)` accounting for the role's direction: an inverse role swaps
/// subject/object so the underlying NAMED property is always stored forward.
fn role_atom(role: &Role, s: Term, o: Term) -> Atom {
    if role.inverse {
        Atom::Role {
            role: Role::named(role.iri.clone()),
            s: o,
            o: s,
        }
    } else {
        Atom::Role {
            role: role.clone(),
            s,
            o,
        }
    }
}

/// For an existential-generating inclusion `B ⊑ ∃gen_role`, decide whether the role atom
/// `R(s,o)` (role direction in `role`) is rewritable to `B(x)`, returning the SURVIVING bound
/// term `x` if so. Requires the FILLER (the position the existential binds) to be `_`-eligible.
///
/// Normalise the atom to the forward named property first: if `role` is forward, the pair is
/// (s, o); if inverse, (o, s). Then `∃P` (gen_role forward) binds the OBJECT — so object must be
/// `_`, surviving term is the subject; `∃P⁻` (gen_role inverse) binds the SUBJECT — subject must
/// be `_`, surviving term is the object.
fn applicable_exists_super(
    cq: &Cq,
    role: &Role,
    s: &Term,
    o: &Term,
    gen_role: &Role,
) -> Option<Term> {
    // Same NAMED property required.
    if role.iri != gen_role.iri {
        return None;
    }
    // Forward (subj, obj) of the underlying named property P.
    let (subj, obj) = if role.inverse { (o, s) } else { (s, o) };
    if gen_role.inverse {
        // ∃P⁻ binds the SUBJECT of P: subject must be `_`, surviving term is the object.
        if underscore_eligible(cq, subj) {
            Some(obj.clone())
        } else {
            None
        }
    } else {
        // ∃P binds the OBJECT of P: object must be `_`, surviving term is the subject.
        if underscore_eligible(cq, obj) {
            Some(subj.clone())
        } else {
            None
        }
    }
}

/// Build the rewritten atom for `B ⊑ ∃R` consumed on a role atom, where `arg` is the surviving
/// bound term: if B is a named class → `B(arg)`; if B is `∃S` → `S(arg, _)`.
fn class_or_role_basic(b: &Basic, arg: Term, fresh: &mut Fresh) -> Atom {
    match b {
        Basic::Class(c) => Atom::Class {
            class: c.clone(),
            arg,
        },
        Basic::Exists(role) => role_atom(role, arg, fresh.next()),
    }
}

/// Apply a role inclusion `r1 ⊑ r2` backward to atom `R(s,o)` (role direction in `role`):
/// if the atom's role is `r2` (same direction), rewrite to `r1`; if it is `r2⁻`, rewrite to
/// `r1⁻`. Inverse directions are handled by comparing the (iri, inverse) pair both ways.
fn apply_role_inclusion(role: &Role, s: &Term, o: &Term, r1: &Role, r2: &Role) -> Option<Atom> {
    if role == r2 {
        // Same direction: replace role with r1, keep (s,o), then forward-normalise.
        Some(role_atom(r1, s.clone(), o.clone()))
    } else if role == &r2.inv() {
        // The atom uses the inverse of r2; the inclusion then yields r1 inverted, with the
        // atom's (s,o) preserved (the double inverse cancels at the named-property level).
        Some(role_atom(&r1.inv(), s.clone(), o.clone()))
    } else {
        None
    }
}

/// REDUCE: attempt to unify atoms `a` and `b` of `cq` (must be the same predicate/arity/role),
/// apply the most-general unifier across the whole query, then ANONYMISE — any variable that, in
/// the unified query, is neither distinguished nor shared becomes a fresh `_`. Returns the
/// reduced CQ, or `None` if the two atoms do not unify.
fn reduce(cq: &Cq, a: usize, b: usize) -> Option<Cq> {
    let pairs = atom_unify_pairs(&cq.atoms[a], &cq.atoms[b])?;
    // Build a substitution Key -> Term via union-find-free direct binding (DL-Lite atoms are
    // binary so the substitution is tiny). A const-vs-different-const clash fails the unify.
    // Keying on BOTH Var names and Unbound ids keeps the unifier sound when an `_` meets a
    // constant (the `_` binds to the constant — the more-specific term wins).
    //
    // SOUNDNESS (sq-g19x0 fix): DISTINGUISHED (answer) variables are RIGID in the MGU — reduce
    // may NOT identify two distinct answer variables, nor bind an answer variable to a constant.
    // Doing so would change the projection's meaning (it answers a *different*, more-constrained
    // query: `?x = ?y`), so the merged CQ would unsoundly drop the original two-distinct-column
    // answers. Standard PerfectRef reduce exists only to collapse NON-distinguished shared
    // variables and thereby re-enable existential rewrites; we enforce that by freezing the answer
    // vars. (Bug caught by oracle case 16: `SELECT ?x ?y { ?x a B . ?y a B }` must keep the 2×2
    // product, not collapse `?x`,`?y`.)
    let answer: FxHashSet<&str> = cq.answer.iter().map(|s| s.as_str()).collect();
    let mut subst: Subst = Subst::default();
    for (x, y) in pairs {
        if !unify_terms(&x, &y, &mut subst, &answer) {
            return None;
        }
    }
    // Apply substitution to every atom, then drop atom `b` (now equal to `a` after unification).
    let mut atoms: Vec<Atom> = cq
        .atoms
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != b)
        .map(|(_, at)| apply_subst_atom(at, &subst))
        .collect();
    let answer = cq.answer.clone();
    // Anonymise: a Var that is non-distinguished and now occurs only once → `_`.
    anonymise(&mut atoms, &answer);
    Some(Cq::normalised(atoms, answer))
}

/// The position-wise term pairs that must unify for two atoms to be unifiable (same predicate /
/// same role direction). `None` if the atoms are not the same predicate shape.
fn atom_unify_pairs(a: &Atom, b: &Atom) -> Option<Vec<(Term, Term)>> {
    match (a, b) {
        (
            Atom::Class { class: c1, arg: a1 },
            Atom::Class { class: c2, arg: a2 },
        ) if c1 == c2 => Some(vec![(a1.clone(), a2.clone())]),
        (
            Atom::Role {
                role: r1,
                s: s1,
                o: o1,
            },
            Atom::Role {
                role: r2,
                s: s2,
                o: o2,
            },
        ) if r1 == r2 => Some(vec![(s1.clone(), s2.clone()), (o1.clone(), o2.clone())]),
        _ => None,
    }
}

/// A substitution key: a named variable or an unbound `_` (by id). Keying on both keeps the
/// unifier sound when an `_` is unified against a constant or another variable.
#[derive(Clone, PartialEq, Eq)]
enum Key {
    Var(String),
    Unbound(u32),
}

/// A most-general-unifier substitution: an ordered list of `Key -> Term` bindings, resolved to a
/// fixpoint. Ordered (not a map) so later bindings shadow earlier ones during resolution.
#[derive(Default)]
struct Subst(Vec<(Key, Term)>);

impl Subst {
    fn bind(&mut self, k: Key, t: Term) {
        self.0.push((k, t));
    }
}

/// The key a (possibly bindable) term occupies, or `None` for a rigid constant.
/// `Literal` is treated like `Const` — rigid, no key. sq-pbz04.3.1.
fn key_of(t: &Term) -> Option<Key> {
    match t {
        Term::Var(v) => Some(Key::Var(v.clone())),
        Term::Unbound(id) => Some(Key::Unbound(*id)),
        Term::Const(_) => None,
        // B2: literals are rigid constants — no substitution key.
        Term::Lit(..) => None,
    }
}

/// Unify two terms into `subst`, treating `Var` and `Unbound` as unifiable variables and `Const`
/// as rigid. Returns false on a const/const clash. (A tiny, total unifier — DL-Lite has no
/// function symbols, so no occurs-check is needed.) When a variable/`_` meets a more-specific
/// term, it binds to it (so `_` ↦ const is recorded, never silently dropped).
///
/// DISTINGUISHED (answer) variables in `answer` are RIGID (like constants): a distinguished var
/// unifies only with itself or with a bindable NON-distinguished term that then binds TO the
/// distinguished var. Two distinct distinguished vars — or a distinguished var vs a constant — do
/// NOT unify (returns false), because identifying answer columns changes the query's meaning and
/// would unsoundly drop answers (sq-g19x0).
fn unify_terms(x: &Term, y: &Term, subst: &mut Subst, answer: &FxHashSet<&str>) -> bool {
    let xr = resolve(x, subst);
    let yr = resolve(y, subst);
    if xr == yr {
        return true;
    }
    let x_rigid = is_rigid(&xr, answer);
    let y_rigid = is_rigid(&yr, answer);
    match (x_rigid, y_rigid) {
        // Two rigid terms that are not equal (the `xr == yr` early-return failed) cannot unify:
        // const≠const, distinguished≠distinguished, or distinguished≠const.
        (true, true) => false,
        // A rigid left term (const or distinguished var): bind the bindable right term TO it.
        (true, false) => {
            // `yr` is bindable (non-rigid Var or Unbound).
            subst.bind(key_of(&yr).expect("non-rigid term is bindable"), xr);
            true
        }
        // A rigid right term: bind the bindable left term TO it.
        (false, true) => {
            subst.bind(key_of(&xr).expect("non-rigid term is bindable"), yr);
            true
        }
        // Both bindable (non-distinguished Var/Unbound on each side): bind left to right.
        (false, false) => {
            subst.bind(key_of(&xr).expect("non-rigid term is bindable"), yr);
            true
        }
    }
}

/// A term is RIGID under the MGU iff it is a constant (IRI or literal) OR a distinguished
/// (answer) variable. Rigid terms may not be rebound; two distinct rigid terms do not unify.
/// sq-pbz04.3.1: `Literal` added as rigid (like `Const`).
fn is_rigid(t: &Term, answer: &FxHashSet<&str>) -> bool {
    match t {
        Term::Const(_) => true,
        Term::Var(v) => answer.contains(v.as_str()),
        Term::Unbound(_) => false,
        // B2: a literal constant is rigid — two distinct literals do not unify, and a literal
        // cannot be bound to a generated witness.
        Term::Lit(..) => true,
    }
}

fn resolve(t: &Term, subst: &Subst) -> Term {
    let mut cur = t.clone();
    // Follow key bindings to a fixpoint (linear; chains are short, no cycles without occurs).
    loop {
        let Some(k) = key_of(&cur) else {
            return cur; // a constant resolves to itself.
        };
        match subst.0.iter().rev().find(|(key, _)| *key == k) {
            Some((_, to)) => {
                let next = to.clone();
                if next == cur {
                    return cur;
                }
                cur = next;
            }
            None => return cur,
        }
    }
}

fn apply_subst_atom(a: &Atom, subst: &Subst) -> Atom {
    match a {
        Atom::Class { class, arg } => Atom::Class {
            class: class.clone(),
            arg: resolve(arg, subst),
        },
        Atom::Role { role, s, o } => Atom::Role {
            role: role.clone(),
            s: resolve(s, subst),
            o: resolve(o, subst),
        },
    }
}

/// Anonymise: replace every NON-distinguished `Var` that occurs exactly once across `atoms` with
/// a fresh `_` (Unbound). This is the τ step that can re-enable existential rewrites after a
/// reduce. Fresh ids here are local and large to avoid colliding with the rewriter's counter.
fn anonymise(atoms: &mut [Atom], answer: &[String]) {
    use rustc_hash::FxHashMap;
    let mut counts: FxHashMap<String, usize> = FxHashMap::default();
    for a in atoms.iter() {
        for_each_var(a, |v| *counts.entry(v.to_string()).or_default() += 1);
    }
    let mut next_id = 1_000_000u32; // local fresh range, distinct from the rewriter seed.
    let mut rename: FxHashMap<String, Term> = FxHashMap::default();
    for (v, c) in &counts {
        if *c == 1 && !answer.iter().any(|a| a == v) {
            rename.insert(v.clone(), Term::Unbound(next_id));
            next_id += 1;
        }
    }
    if rename.is_empty() {
        return;
    }
    for a in atoms.iter_mut() {
        map_terms(a, |t| {
            if let Term::Var(v) = t {
                if let Some(r) = rename.get(v) {
                    return r.clone();
                }
            }
            t.clone()
        });
    }
}

fn for_each_var(a: &Atom, mut f: impl FnMut(&str)) {
    let mut go = |t: &Term| {
        if let Term::Var(v) = t {
            f(v);
        }
    };
    match a {
        Atom::Class { arg, .. } => go(arg),
        Atom::Role { s, o, .. } => {
            go(s);
            go(o);
        }
    }
}

fn map_terms(a: &mut Atom, mut f: impl FnMut(&Term) -> Term) {
    match a {
        Atom::Class { arg, .. } => *arg = f(arg),
        Atom::Role { s, o, .. } => {
            *s = f(s);
            *o = f(o);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dllite::ConceptInclusion;

    fn class(c: &str, v: &str) -> Atom {
        Atom::Class {
            class: c.to_string(),
            arg: Term::Var(v.to_string()),
        }
    }

    #[test]
    fn subclass_rewrite_adds_disjunct() {
        // A ⊑ B ; query B(?x) ⇒ UCQ { B(?x), A(?x) }.
        let mut tbox = TBox::default();
        tbox.concept_incl.push(ConceptInclusion {
            sub: Basic::Class("A".into()),
            sup: "B".into(),
        });
        let ucq = perfect_ref(vec![class("B", "x")], vec!["x".into()], &tbox);
        assert_eq!(ucq.len(), 2, "original + one rewrite");
        assert!(ucq.iter().any(|q| q.atoms == vec![class("A", "x")]));
    }

    #[test]
    fn domain_rewrite_introduces_role() {
        // ∃R ⊑ A ; query A(?x) ⇒ add R(?x, _).
        let mut tbox = TBox::default();
        tbox.concept_incl.push(ConceptInclusion {
            sub: Basic::Exists(Role::named("R")),
            sup: "A".into(),
        });
        let ucq = perfect_ref(vec![class("A", "x")], vec!["x".into()], &tbox);
        assert!(ucq.iter().any(|q| matches!(
            q.atoms.as_slice(),
            [Atom::Role { role, s: Term::Var(v), o }] if role.iri == "R" && v == "x" && o.is_unbound()
        )));
    }

    fn role(r: &str, s: Term, o: Term) -> Atom {
        Atom::Role {
            role: Role::named(r),
            s,
            o,
        }
    }

    #[test]
    fn unify_unbound_with_const_binds_to_const() {
        // The hardened-unifier soundness case: `_` unified against a constant must bind to the
        // constant (not silently drop the constraint). Reduce two role atoms R(?x, _) and
        // R(?x, :c) — unifying the objects (_ vs :c) ⇒ the surviving atom is R(?x, :c).
        let cq = Cq::normalised(
            vec![
                role("R", Term::Var("x".into()), Term::Unbound(0)),
                role("R", Term::Var("x".into()), Term::Const("c".into())),
            ],
            vec!["x".into()],
        );
        let reduced = reduce(&cq, 0, 1).expect("the two R atoms unify");
        // After reduce the single surviving atom must keep the constant object :c.
        assert!(
            reduced.atoms.iter().any(|a| matches!(
                a,
                Atom::Role { role, s: Term::Var(v), o: Term::Const(c) }
                    if role.iri == "R" && v == "x" && c == "c"
            )),
            "reduce must preserve the constant object; got {:?}",
            reduced.atoms
        );
    }

    #[test]
    fn reduce_const_clash_fails() {
        // Two atoms with DIFFERENT constants in the same position do NOT unify (reduce = None).
        let cq = Cq::normalised(
            vec![
                class_const("A", "c1"),
                class_const("A", "c2"),
            ],
            vec![],
        );
        assert!(reduce(&cq, 0, 1).is_none(), "distinct constants must not unify");
    }

    #[test]
    fn reduce_must_not_identify_two_distinguished_vars() {
        // SOUNDNESS (sq-g19x0): reduce must NOT unify two atoms when that would identify two
        // DISTINCT distinguished (answer) variables — doing so answers a different query (?x=?y)
        // and unsoundly drops the two-distinct-column answers. `B(?x)` and `B(?y)` with BOTH ?x
        // and ?y distinguished must NOT reduce.
        let cq = Cq::normalised(
            vec![class("B", "x"), class("B", "y")],
            vec!["x".into(), "y".into()],
        );
        assert!(
            reduce(&cq, 0, 1).is_none(),
            "reduce must not identify two distinct distinguished variables"
        );
    }

    #[test]
    fn reduce_still_collapses_nondistinguished_shared_var() {
        // The LEGITIMATE reduce: `R(?x,?y)` and `R(?x,?z)` with ONLY ?x distinguished — unifying
        // the objects ?y,?z (both non-distinguished) is sound and re-enables existential rewrites.
        let cq = Cq::normalised(
            vec![
                role("R", Term::Var("x".into()), Term::Var("y".into())),
                role("R", Term::Var("x".into()), Term::Var("z".into())),
            ],
            vec!["x".into()],
        );
        let reduced = reduce(&cq, 0, 1).expect("non-distinguished objects must unify");
        // One surviving R atom; the merged object is non-distinguished+single → anonymised to `_`.
        assert_eq!(reduced.atoms.len(), 1, "the two R atoms collapse to one");
        assert!(matches!(
            reduced.atoms[0],
            Atom::Role { s: Term::Var(ref v), o: Term::Unbound(_), .. } if v == "x"
        ));
    }

    #[test]
    fn reduce_must_not_bind_distinguished_var_to_const() {
        // A distinguished var is rigid vs a constant too: `A(?x)` and `A(:c)` with ?x distinguished
        // must NOT reduce (it would force ?x = :c, answering a different query).
        let cq = Cq::normalised(
            vec![
                class("A", "x"),
                class_const("A", "c"),
            ],
            vec!["x".into()],
        );
        assert!(
            reduce(&cq, 0, 1).is_none(),
            "a distinguished var must not be bound to a constant by reduce"
        );
    }

    fn class_const(c: &str, k: &str) -> Atom {
        Atom::Class {
            class: c.to_string(),
            arg: Term::Const(k.to_string()),
        }
    }
}
