// 🤖 SPARQ agent [SONNET-4.6]. Issue #2503 ("make sure tableau reasoning is properly
// implemented") — a RANDOMISED CROSS-CHECK of the L3 ALCH tableau (`src/tableau.rs`)
// against an INDEPENDENT oracle, plus the logical identities the tableau must respect.
//
// WHY THIS EXISTS. `tests/tableau.rs` pins hand-derived verdicts on hand-written
// ontologies: it proves the tableau agrees with its author on the cases its author
// thought of. The module's soundness/completeness argument (tableau module docs §4/§5)
// is a PROOF ABOUT THE CALCULUS, not about this Rust code. Nothing in the suite searched
// for a disagreement between the two. This file does.
//
// THE TWO ARMS
// ------------
// (1) MODEL CROSS-CHECK — a brute-force finite-model oracle (`oracle` below) that
//     implements the W3C OWL 2 DIRECT SEMANTICS for the ALCH fragment DIRECTLY over the
//     structural model: it enumerates every interpretation over a domain of size ≤ N and
//     evaluates each `Axiom` by its own definition. It shares NO code with the tableau —
//     no NNF, no internalisation, no desugaring.
//
//     The oracle is a ONE-SIDED witness and is used only in the direction where that is
//     valid: EXHIBITING a model refutes `Unsatisfiable`. Failing to find one within the
//     size bound proves nothing (ALCH has the finite model property, but nothing bounds a
//     model to the domain sizes enumerated here), so no assertion is made in that
//     direction. So this arm sees UNSOUNDNESS only — a tableau that infers too much. It is
//     what would catch a spurious clash, an over-eager blocking condition, or a silent
//     unique-name assumption over the two individuals (the oracle lets both denote the same
//     element). It CANNOT see incompleteness; that is arm (2)'s job, and the split is
//     load-bearing rather than cosmetic — a desugaring bug, for instance, drops constraints
//     and so only ever yields `Satisfiable`, which no one-sided model oracle can refute.
//
// (2) LOGICAL IDENTITIES — metamorphic properties that are two-sided, and therefore catch
//     the INCOMPLETENESS the oracle cannot: a wrongly-`Unsatisfiable` verdict is caught by
//     excluded middle (§M4), a wrongly-`Satisfiable` one by the contradiction (§M3) and by
//     desugaring invariance (§M8), which re-expresses `equivalentClass` / `disjointWith` /
//     `domain` / `range` as the GCIs the semantics defines them to be and demands the same
//     verdict — routing the same ontology through `preprocess`'s dedicated arms and through
//     its generic GCI path. These need no oracle, so they run over a much larger sample.
//
// DETERMINISM. Everything here is seeded and count-bounded: a fixed LCG (no `rand`
// dependency, no wall clock), fixed budgets, fixed generation grammar. The suite is a
// pure function of the seeds below — a failure reproduces exactly, and re-running it
// never reports a *different* counterexample.
//
// SCOPE. This exercises the DEFAULT feature state (the ALCH fragment). The opt-in
// `dl_transitive` ∀₊-rule is pinned by the dedicated tests in `tests/tableau.rs`; the
// oracle would need transitive-closure role semantics to cover it.
//
// NON-VACUITY. Budget exhaustion is a legitimate fail-closed abstention, so an `Unknown`
// iteration is skipped rather than failed — which would let the whole suite pass
// vacuously if the tableau ever started abstaining everywhere. Every test therefore ends by
// asserting a COVERAGE FLOOR. Arm (1): enough iterations decided, plus — counted SEPARATELY
// for each of the two APIs it cross-checks, `consistency` and `class_satisfiability` — the
// oracle both DID and DID NOT exhibit a witness often enough (otherwise the generator is
// emitting only trivially-consistent ontologies, or only trivially-inhabited classes, and
// that API's assertion never bites). The split matters: a single shared floor would let the
// class-satisfiability assertion go unexercised while the ontology-model counters still
// cleared it, so a tableau answering `Unsatisfiable` to every class query would go
// unchallenged. Arm (2): enough iterations decided, and both `Satisfiable` and
// `Unsatisfiable` observed.

use sparq_reason_dl::model::{Axiom, ClassExpression as CE, ObjectPropertyExpression as OPE};
use sparq_reason_dl::tableau::{class_satisfiability, consistency, Budget, Verdict};
use sparq_reason_dl::Ontology;

// -------------------------------------------------------------------------------------------
// Deterministic PRNG — a fixed 64-bit LCG (Knuth's MMIX constants), high bits only.
// -------------------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    /// Uniform-enough in `0..n` for a test generator (`n` is always a small constant).
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

// -------------------------------------------------------------------------------------------
// Signature + random ALCH ontology generation
// -------------------------------------------------------------------------------------------

/// The finite vocabulary a generated ontology (and the oracle) ranges over. Kept tiny so
/// the oracle's enumeration stays exhaustive and cheap.
struct Sig {
    classes: &'static [u32],
    roles: &'static [u32],
    individuals: &'static [u32],
}

/// Two roles + two individuals: exercises the `subPropertyOf` closure and the
/// no-unique-name-assumption ABox. Paired with a domain bound of 2.
const WIDE: Sig = Sig {
    classes: &[1, 2],
    roles: &[10, 11],
    individuals: &[100, 101],
};

/// One role, one individual: a smaller interpretation space, so the oracle can afford a
/// domain of 3 — deep enough that a tableau branch must block to terminate.
const DEEP: Sig = Sig {
    classes: &[1, 2],
    roles: &[10],
    individuals: &[100],
};

fn gen_class(rng: &mut Rng, sig: &Sig, depth: u32) -> CE {
    // At depth 0 only leaves, so generation always terminates.
    let k = if depth == 0 { rng.below(4) } else { rng.below(9) };
    match k {
        0 | 1 => CE::Class(sig.classes[rng.below(sig.classes.len())]),
        2 => CE::Thing,
        3 => CE::Nothing,
        4 => CE::ObjectIntersectionOf(vec![
            gen_class(rng, sig, depth - 1),
            gen_class(rng, sig, depth - 1),
        ]),
        5 => CE::ObjectUnionOf(vec![
            gen_class(rng, sig, depth - 1),
            gen_class(rng, sig, depth - 1),
        ]),
        6 => CE::ObjectComplementOf(Box::new(gen_class(rng, sig, depth - 1))),
        7 => CE::some(
            sig.roles[rng.below(sig.roles.len())],
            gen_class(rng, sig, depth - 1),
        ),
        _ => CE::only(
            sig.roles[rng.below(sig.roles.len())],
            gen_class(rng, sig, depth - 1),
        ),
    }
}

fn gen_axiom(rng: &mut Rng, sig: &Sig) -> Axiom {
    let role = |rng: &mut Rng| OPE::ObjectProperty(sig.roles[rng.below(sig.roles.len())]);
    let ind = |rng: &mut Rng| sig.individuals[rng.below(sig.individuals.len())];
    match rng.below(8) {
        0 | 1 => Axiom::SubClassOf {
            sub: gen_class(rng, sig, 2),
            sup: gen_class(rng, sig, 2),
        },
        2 => Axiom::EquivalentClasses(gen_class(rng, sig, 1), gen_class(rng, sig, 2)),
        3 => Axiom::DisjointClasses(gen_class(rng, sig, 1), gen_class(rng, sig, 2)),
        4 => Axiom::ObjectPropertyDomain {
            property: role(rng),
            domain: gen_class(rng, sig, 1),
        },
        5 => Axiom::ObjectPropertyRange {
            property: role(rng),
            range: gen_class(rng, sig, 1),
        },
        6 => Axiom::ClassAssertion {
            class: gen_class(rng, sig, 2),
            individual: ind(rng),
        },
        _ => {
            if sig.roles.len() > 1 && rng.below(2) == 0 {
                // A role inclusion between two DISTINCT roles (a self-inclusion would be
                // vacuous, and the closure is already reflexive).
                Axiom::SubObjectPropertyOf {
                    sub: OPE::ObjectProperty(sig.roles[0]),
                    sup: OPE::ObjectProperty(sig.roles[1]),
                }
            } else {
                Axiom::ObjectPropertyAssertion {
                    property: role(rng),
                    source: ind(rng),
                    target: ind(rng),
                }
            }
        }
    }
}

fn gen_ontology(rng: &mut Rng, sig: &Sig, max_axioms: usize) -> Ontology {
    let n = 1 + rng.below(max_axioms);
    Ontology {
        axioms: (0..n).map(|_| gen_axiom(rng, sig)).collect(),
    }
}

// -------------------------------------------------------------------------------------------
// The oracle: brute-force OWL 2 Direct Semantics over a finite domain
// -------------------------------------------------------------------------------------------
//
// An interpretation over `Δ = 0..n`: a subset of Δ per class name, a subset of Δ×Δ per role
// name, and an element of Δ per individual name (NO unique-name assumption — two individuals
// may denote the same element, exactly as OWL 2 has it). Written straight from the W3C
// Direct Semantics tables; it deliberately does NOT reuse the tableau's NNF, internalisation,
// or desugaring, so agreement is real evidence rather than a tautology.

/// One finite interpretation over the domain `0..n`.
struct Interp<'a> {
    sig: &'a Sig,
    n: usize,
    /// Per element, a bitmask over `sig.classes` positions.
    classes: Vec<u32>,
    /// Per `sig.roles` position, an `n * n` adjacency matrix in row-major order.
    roles: Vec<Vec<bool>>,
    /// Per `sig.individuals` position, the denoted element.
    inds: Vec<usize>,
}

impl Interp<'_> {
    fn class_pos(&self, id: u32) -> usize {
        self.sig
            .classes
            .iter()
            .position(|&c| c == id)
            .expect("generated ontologies only mention signature classes")
    }

    fn role_pos(&self, id: u32) -> usize {
        self.sig
            .roles
            .iter()
            .position(|&r| r == id)
            .expect("generated ontologies only mention signature roles")
    }

    fn ind_elem(&self, id: u32) -> usize {
        let p = self
            .sig
            .individuals
            .iter()
            .position(|&i| i == id)
            .expect("generated ontologies only mention signature individuals");
        self.inds[p]
    }

    fn has_edge(&self, role: u32, from: usize, to: usize) -> bool {
        self.roles[self.role_pos(role)][from * self.n + to]
    }

    /// `element ∈ ce^I` — the Direct-Semantics interpretation function for class expressions.
    fn holds(&self, element: usize, ce: &CE) -> bool {
        match ce {
            CE::Class(id) => self.classes[element] & (1 << self.class_pos(*id)) != 0,
            CE::Thing => true,
            CE::Nothing => false,
            CE::ObjectIntersectionOf(ms) => ms.iter().all(|m| self.holds(element, m)),
            CE::ObjectUnionOf(ms) => ms.iter().any(|m| self.holds(element, m)),
            CE::ObjectComplementOf(inner) => !self.holds(element, inner),
            CE::ObjectSomeValuesFrom(p, filler) => (0..self.n)
                .any(|t| self.has_edge(p.named(), element, t) && self.holds(t, filler)),
            CE::ObjectAllValuesFrom(p, filler) => (0..self.n)
                .all(|t| !self.has_edge(p.named(), element, t) || self.holds(t, filler)),
        }
    }

    /// `I ⊨ axiom`, one arm per axiom kind, straight from the Direct Semantics.
    fn models_axiom(&self, axiom: &Axiom) -> bool {
        let all = |f: &dyn Fn(usize) -> bool| (0..self.n).all(f);
        match axiom {
            Axiom::SubClassOf { sub, sup } => {
                all(&|e| !self.holds(e, sub) || self.holds(e, sup))
            }
            Axiom::EquivalentClasses(l, r) => {
                all(&|e| self.holds(e, l) == self.holds(e, r))
            }
            Axiom::DisjointClasses(l, r) => {
                all(&|e| !(self.holds(e, l) && self.holds(e, r)))
            }
            Axiom::SubObjectPropertyOf { sub, sup } => (0..self.n).all(|x| {
                (0..self.n)
                    .all(|y| !self.has_edge(sub.named(), x, y) || self.has_edge(sup.named(), x, y))
            }),
            Axiom::ObjectPropertyDomain { property, domain } => (0..self.n).all(|x| {
                (0..self.n)
                    .all(|y| !self.has_edge(property.named(), x, y) || self.holds(x, domain))
            }),
            Axiom::ObjectPropertyRange { property, range } => (0..self.n).all(|x| {
                (0..self.n)
                    .all(|y| !self.has_edge(property.named(), x, y) || self.holds(y, range))
            }),
            Axiom::ClassAssertion { class, individual } => {
                self.holds(self.ind_elem(*individual), class)
            }
            Axiom::ObjectPropertyAssertion {
                property,
                source,
                target,
            } => self.has_edge(
                property.named(),
                self.ind_elem(*source),
                self.ind_elem(*target),
            ),
            // The default feature state has no further variants; an added one must be given
            // its own oracle arm rather than silently treated as satisfied.
            #[allow(unreachable_patterns)]
            _ => unreachable!("oracle must model every Axiom variant explicitly"),
        }
    }
}

/// Search every interpretation over a domain of size `1..=max_n` for a model of `onto` in
/// which `witness` (when given) is non-empty. `true` means a model was EXHIBITED.
///
/// One-sided by construction: `false` means only "no model this small", never "no model".
fn oracle(sig: &Sig, onto: &Ontology, witness: Option<&CE>, max_n: usize) -> bool {
    let (nc, nr, ni) = (sig.classes.len(), sig.roles.len(), sig.individuals.len());
    for n in 1..=max_n {
        let class_combos = 1u64 << (nc * n);
        let role_combos = 1u64 << (n * n);
        let mut total = class_combos;
        for _ in 0..nr {
            total *= role_combos;
        }
        for _ in 0..ni {
            total *= n as u64;
        }
        // Keeps the oracle's cost a reviewable constant rather than an accident of the
        // signature: a signature that blows this up must be re-sized deliberately.
        assert!(
            total <= 1 << 21,
            "oracle enumeration too large ({}) — shrink the signature or the domain bound",
            total
        );

        for idx in 0..total {
            let mut rest = idx;
            let cbits = rest % class_combos;
            rest /= class_combos;
            let classes: Vec<u32> = (0..n)
                .map(|e| ((cbits >> (e * nc)) & ((1 << nc) - 1)) as u32)
                .collect();
            let roles: Vec<Vec<bool>> = (0..nr)
                .map(|_| {
                    let bits = rest % role_combos;
                    rest /= role_combos;
                    (0..n * n).map(|k| bits & (1 << k) != 0).collect()
                })
                .collect();
            let inds: Vec<usize> = (0..ni)
                .map(|_| {
                    let e = rest % n as u64;
                    rest /= n as u64;
                    e as usize
                })
                .collect();

            let i = Interp {
                sig,
                n,
                classes,
                roles,
                inds,
            };
            if !onto.axioms().iter().all(|a| i.models_axiom(a)) {
                continue;
            }
            match witness {
                None => return true,
                Some(c) => {
                    if (0..n).any(|e| i.holds(e, c)) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

// -------------------------------------------------------------------------------------------
// Shared helpers
// -------------------------------------------------------------------------------------------

/// A deterministic count budget generous enough to decide almost every generated case, and
/// small enough that the pathological ones abstain promptly instead of stalling the suite.
const BUDGET: Budget = Budget {
    max_nodes: 2_000,
    max_rule_applications: 40_000,
};

fn not(c: &CE) -> CE {
    CE::ObjectComplementOf(Box::new(c.clone()))
}

/// An individual id used by no generated ontology, for the `class_satisfiability` reduction.
const FRESH_INDIVIDUAL: u32 = 999;

/// Rewrite one axiom into the GCIs the W3C Direct Semantics DEFINES it to mean.
///
/// This is the definition, taken from the semantics — not a copy of the tableau's
/// `preprocess`. `preprocess` handles [`Axiom::EquivalentClasses`] /
/// [`Axiom::DisjointClasses`] / [`Axiom::ObjectPropertyDomain`] /
/// [`Axiom::ObjectPropertyRange`] in dedicated arms, whereas the rewritten forms below go
/// through its generic `SubClassOf` path. Feeding both to the tableau and demanding the same
/// verdict (§M8) is what makes a wrong desugaring observable: a bug confined to one of those
/// arms shows up as a disagreement, which no one-sided model oracle can see (a too-weak
/// desugaring loses constraints, and losing constraints only ever yields `Satisfiable`).
fn desugar(axiom: &Axiom) -> Vec<Axiom> {
    let gci = |sub: CE, sup: CE| Axiom::SubClassOf { sub, sup };
    match axiom {
        // C ≡ D  ⟺  C ⊑ D and D ⊑ C.
        Axiom::EquivalentClasses(l, r) => {
            vec![gci(l.clone(), r.clone()), gci(r.clone(), l.clone())]
        }
        // Disjoint(C, D)  ⟺  C^I ∩ D^I = ∅  ⟺  C ⊓ D ⊑ ⊥.
        Axiom::DisjointClasses(l, r) => vec![gci(
            CE::ObjectIntersectionOf(vec![l.clone(), r.clone()]),
            CE::Nothing,
        )],
        // Domain(R, C)  ⟺  every subject of an R-edge is a C  ⟺  ∃R.⊤ ⊑ C.
        Axiom::ObjectPropertyDomain { property, domain } => vec![gci(
            CE::some(property.named(), CE::Thing),
            domain.clone(),
        )],
        // Range(R, C)  ⟺  every object of an R-edge is a C  ⟺  ⊤ ⊑ ∀R.C.
        Axiom::ObjectPropertyRange { property, range } => vec![gci(
            CE::Thing,
            CE::only(property.named(), range.clone()),
        )],
        // Already primitive for the tableau (GCI / RBox / ABox).
        other => vec![other.clone()],
    }
}

// -------------------------------------------------------------------------------------------
// Arm 1 — cross-check against the independent model oracle
// -------------------------------------------------------------------------------------------

/// How often each oracle arm actually bit, so the floors below can prove non-vacuity
/// SEPARATELY for the two APIs under cross-check.
///
/// The two arms are counted apart on purpose: a floor on the ontology-model arm alone would
/// let the class-satisfiability assertion go silently unexercised (a generator drifting
/// towards empty classes, or a witness search rendered ineffective, would leave a tableau
/// that answers `Unsatisfiable` to every class query unchallenged while `decided` /
/// `onto_model` / `onto_no_model` all still cleared their floors).
#[derive(Default)]
struct Coverage {
    /// Iterations where BOTH verdicts were decided (neither abstained on budget).
    decided: usize,
    /// `consistency` arm: the oracle exhibited a model of the ontology.
    onto_model: usize,
    /// `consistency` arm: no model within the domain bound.
    onto_no_model: usize,
    /// `class_satisfiability` arm: the oracle exhibited a model with a non-empty class.
    class_witness: usize,
    /// `class_satisfiability` arm: no such witness within the domain bound.
    class_no_witness: usize,
}

/// Runs one signature through the oracle and returns the per-arm [`Coverage`].
fn crosscheck(sig: &Sig, seed: u64, iterations: usize, max_n: usize) -> Coverage {
    let mut rng = Rng::new(seed);
    let mut cov = Coverage::default();

    for iter in 0..iterations {
        let onto = gen_ontology(&mut rng, sig, 4);
        let class = gen_class(&mut rng, sig, 2);

        let v_onto = consistency(&onto, BUDGET);
        let v_class = class_satisfiability(&class, &onto, BUDGET);

        // A model the oracle can EXHIBIT refutes `Unsatisfiable` — the tableau would be
        // unsound (module docs §4). `Unknown` is a legitimate fail-closed abstention.
        if oracle(sig, &onto, None, max_n) {
            cov.onto_model += 1;
            assert_ne!(
                v_onto,
                Verdict::Unsatisfiable,
                "iter {}: oracle exhibited a model but the tableau called the ontology \
                 inconsistent — {:?}",
                iter,
                onto
            );
        } else {
            cov.onto_no_model += 1;
        }

        if oracle(sig, &onto, Some(&class), max_n) {
            cov.class_witness += 1;
            assert_ne!(
                v_class,
                Verdict::Unsatisfiable,
                "iter {}: oracle exhibited a model with a non-empty {:?} but the tableau \
                 called the class unsatisfiable — {:?}",
                iter,
                class,
                onto
            );
        } else {
            cov.class_no_witness += 1;
        }

        if !v_onto.is_unknown() && !v_class.is_unknown() {
            cov.decided += 1;
        }
    }
    cov
}

#[test]
fn oracle_never_contradicts_an_unsatisfiable_verdict_wide() {
    // Two roles + two individuals (role hierarchy + no-UNA ABox), domain ≤ 2.
    let cov = crosscheck(&WIDE, 0x5EED_0001, 120, 2);
    assert!(
        cov.decided >= 90,
        "too few decided cases ({}) — test near-vacuous",
        cov.decided
    );

    // `consistency` arm.
    assert!(
        cov.onto_model >= 20,
        "oracle exhibited a model in only {} cases",
        cov.onto_model
    );
    assert!(
        cov.onto_no_model >= 5,
        "oracle found a model in all but {} cases — the generator is producing only \
         trivially-consistent ontologies",
        cov.onto_no_model
    );

    // `class_satisfiability` arm — floored SEPARATELY, so this API's oracle assertion is
    // demonstrably exercised rather than riding on the ontology arm's coverage.
    assert!(
        cov.class_witness >= 20,
        "oracle exhibited a non-empty generated class in only {} cases — the class-\
         satisfiability assertion is near-vacuous",
        cov.class_witness
    );
    assert!(
        cov.class_no_witness >= 5,
        "oracle witnessed a non-empty class in all but {} cases — the generator is producing \
         only trivially-inhabited classes",
        cov.class_no_witness
    );
}

#[test]
fn oracle_never_contradicts_an_unsatisfiable_verdict_deep() {
    // One role + one individual, domain ≤ 3 — deep enough to need blocking.
    let cov = crosscheck(&DEEP, 0x5EED_0002, 60, 3);
    assert!(
        cov.decided >= 45,
        "too few decided cases ({}) — test near-vacuous",
        cov.decided
    );

    // `consistency` arm.
    assert!(
        cov.onto_model >= 10,
        "oracle exhibited a model in only {} cases",
        cov.onto_model
    );
    assert!(
        cov.onto_no_model >= 2,
        "oracle never failed to find a model ({})",
        cov.onto_no_model
    );

    // `class_satisfiability` arm — see the WIDE test for why this is floored separately.
    assert!(
        cov.class_witness >= 10,
        "oracle exhibited a non-empty generated class in only {} cases — the class-\
         satisfiability assertion is near-vacuous",
        cov.class_witness
    );
    assert!(
        cov.class_no_witness >= 2,
        "oracle never failed to witness a non-empty class ({})",
        cov.class_no_witness
    );
}

// -------------------------------------------------------------------------------------------
// Arm 2 — logical identities the tableau must respect (two-sided, no oracle)
// -------------------------------------------------------------------------------------------

/// Apply `f` to every class / role / individual id in the ontology.
fn rename_ontology(onto: &Ontology, f: &dyn Fn(u32) -> u32) -> Ontology {
    Ontology {
        axioms: onto.axioms().iter().map(|a| rename_axiom(a, f)).collect(),
    }
}

fn rename_axiom(axiom: &Axiom, f: &dyn Fn(u32) -> u32) -> Axiom {
    let role = |p: &OPE| OPE::ObjectProperty(f(p.named()));
    match axiom {
        Axiom::SubClassOf { sub, sup } => Axiom::SubClassOf {
            sub: rename_class(sub, f),
            sup: rename_class(sup, f),
        },
        Axiom::EquivalentClasses(l, r) => {
            Axiom::EquivalentClasses(rename_class(l, f), rename_class(r, f))
        }
        Axiom::DisjointClasses(l, r) => {
            Axiom::DisjointClasses(rename_class(l, f), rename_class(r, f))
        }
        Axiom::SubObjectPropertyOf { sub, sup } => Axiom::SubObjectPropertyOf {
            sub: role(sub),
            sup: role(sup),
        },
        Axiom::ObjectPropertyDomain { property, domain } => Axiom::ObjectPropertyDomain {
            property: role(property),
            domain: rename_class(domain, f),
        },
        Axiom::ObjectPropertyRange { property, range } => Axiom::ObjectPropertyRange {
            property: role(property),
            range: rename_class(range, f),
        },
        Axiom::ClassAssertion { class, individual } => Axiom::ClassAssertion {
            class: rename_class(class, f),
            individual: f(*individual),
        },
        Axiom::ObjectPropertyAssertion {
            property,
            source,
            target,
        } => Axiom::ObjectPropertyAssertion {
            property: role(property),
            source: f(*source),
            target: f(*target),
        },
        // `gen_axiom` emits only the variants above, in every feature state — the opt-in
        // `dl_transitive` `TransitiveObjectProperty` is never generated. Failing loudly beats
        // cloning it unrenamed, which would silently make M7 compare two different ontologies.
        #[allow(unreachable_patterns)]
        other => unreachable!("generator emitted an unhandled axiom: {:?}", other),
    }
}

fn rename_class(ce: &CE, f: &dyn Fn(u32) -> u32) -> CE {
    match ce {
        CE::Class(id) => CE::Class(f(*id)),
        CE::Thing | CE::Nothing => ce.clone(),
        CE::ObjectIntersectionOf(ms) => {
            CE::ObjectIntersectionOf(ms.iter().map(|m| rename_class(m, f)).collect())
        }
        CE::ObjectUnionOf(ms) => {
            CE::ObjectUnionOf(ms.iter().map(|m| rename_class(m, f)).collect())
        }
        CE::ObjectComplementOf(inner) => {
            CE::ObjectComplementOf(Box::new(rename_class(inner, f)))
        }
        CE::ObjectSomeValuesFrom(p, filler) => {
            CE::some(f(p.named()), rename_class(filler, f))
        }
        CE::ObjectAllValuesFrom(p, filler) => {
            CE::only(f(p.named()), rename_class(filler, f))
        }
    }
}

#[test]
fn logical_identities_hold_over_random_alch_ontologies() {
    let mut rng = Rng::new(0x5EED_0003);
    let (mut decided, mut sat, mut unsat) = (0usize, 0usize, 0usize);

    for iter in 0..400 {
        let onto = gen_ontology(&mut rng, &WIDE, 4);
        let class = gen_class(&mut rng, &WIDE, 2);

        let v_onto = consistency(&onto, BUDGET);
        let v_class = class_satisfiability(&class, &onto, BUDGET);
        let v_neg = class_satisfiability(&not(&class), &onto, BUDGET);
        let v_and = class_satisfiability(
            &CE::ObjectIntersectionOf(vec![class.clone(), not(&class)]),
            &onto,
            BUDGET,
        );
        let v_or = class_satisfiability(
            &CE::ObjectUnionOf(vec![class.clone(), not(&class)]),
            &onto,
            BUDGET,
        );
        let v_top = class_satisfiability(&CE::Thing, &onto, BUDGET);

        // Fail-closed abstention is not a violation — but it must stay rare (asserted below).
        if [&v_onto, &v_class, &v_neg, &v_and, &v_or, &v_top]
            .iter()
            .any(|v| v.is_unknown())
        {
            continue;
        }
        decided += 1;
        match v_onto {
            Verdict::Satisfiable => sat += 1,
            Verdict::Unsatisfiable => unsat += 1,
            Verdict::Unknown(_) => unreachable!("filtered above"),
        }

        // M1 — ⊤ is satisfiable w.r.t. O exactly when O is consistent (the domain of a
        // Direct-Semantics interpretation is non-empty, so ⊤ is inhabited in every model).
        assert_eq!(v_top, v_onto, "iter {}: M1 broke on {:?}", iter, onto);

        // M2 — the reduction the module documents: `class_satisfiability(C, O)` is
        // `consistency(O ∪ {C(a_fresh)})` for an individual occurring nowhere else.
        let mut reduced = onto.clone();
        reduced.axioms.push(Axiom::ClassAssertion {
            class: class.clone(),
            individual: FRESH_INDIVIDUAL,
        });
        assert_eq!(
            v_class,
            consistency(&reduced, BUDGET),
            "iter {}: M2 (class-sat ≡ consistency of O ∪ {{C(fresh)}}) broke on {:?} / {:?}",
            iter,
            onto,
            class
        );

        // M3 — a contradiction is unsatisfiable w.r.t. EVERY ontology. Catches a wrongly
        // `Satisfiable` verdict (a completeness failure of the clash rules).
        assert_eq!(
            v_and,
            Verdict::Unsatisfiable,
            "iter {}: M3 (C ⊓ ¬C unsatisfiable) broke on {:?} / {:?}",
            iter,
            onto,
            class
        );

        if v_onto == Verdict::Satisfiable {
            // M4 — excluded middle. In any model, every element is in C or in ¬C, so a
            // consistent O makes `C ⊔ ¬C` satisfiable and at least one of C / ¬C
            // satisfiable. Catches a wrongly `Unsatisfiable` verdict.
            assert_eq!(
                v_or,
                Verdict::Satisfiable,
                "iter {}: M4 (C ⊔ ¬C satisfiable) broke on {:?} / {:?}",
                iter,
                onto,
                class
            );
            assert!(
                v_class == Verdict::Satisfiable || v_neg == Verdict::Satisfiable,
                "iter {}: M4 (one of C / ¬C satisfiable) broke on {:?} / {:?}",
                iter,
                onto,
                class
            );
        } else {
            // M5 — w.r.t. an inconsistent ontology EVERY class is unsatisfiable.
            assert_eq!(
                v_class,
                Verdict::Unsatisfiable,
                "iter {}: M5 broke on {:?} / {:?}",
                iter,
                onto,
                class
            );
        }

        // M6 — monotonicity: adding an axiom can never repair an inconsistency.
        let mut extended = onto.clone();
        extended.axioms.push(gen_axiom(&mut rng, &WIDE));
        let v_extended = consistency(&extended, BUDGET);
        if v_onto == Verdict::Unsatisfiable && !v_extended.is_unknown() {
            assert_eq!(
                v_extended,
                Verdict::Unsatisfiable,
                "iter {}: M6 (monotonicity) broke — {:?} grew to {:?}",
                iter,
                onto,
                extended
            );
        }

        // M7 — the verdict depends on the ontology's SHAPE, not on its dict ids: an
        // injective renaming of every class / role / individual must preserve it.
        let renamed = rename_ontology(&onto, &|id| id + 1_000);
        assert_eq!(
            consistency(&renamed, BUDGET),
            v_onto,
            "iter {}: M7 (renaming invariance) broke on {:?}",
            iter,
            onto
        );

        // M8 — desugaring invariance. Replacing every non-primitive axiom by the GCIs the
        // Direct Semantics defines it to mean must not move the verdict. This is the arm
        // that sees a broken `equivalentClass` / `disjointWith` / `domain` / `range`
        // desugaring, because those forms then reach the tableau by two different paths.
        let desugared = Ontology {
            axioms: onto.axioms().iter().flat_map(desugar).collect(),
        };
        let v_desugared = consistency(&desugared, BUDGET);
        if !v_desugared.is_unknown() {
            assert_eq!(
                v_desugared, v_onto,
                "iter {}: M8 (desugaring invariance) broke — {:?} desugars to {:?}",
                iter, onto, desugared
            );
        }
    }

    // Coverage floors — without these the suite would pass vacuously if the tableau
    // started abstaining, or only ever produced one verdict.
    assert!(
        decided >= 300,
        "only {} of 400 iterations were decided — the identities went near-vacuous",
        decided
    );
    assert!(sat >= 30, "only {} Satisfiable verdicts observed", sat);
    assert!(unsat >= 30, "only {} Unsatisfiable verdicts observed", unsat);
}
