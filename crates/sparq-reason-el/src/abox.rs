// [OPUS-4.8] sq-pbz04.2.5 (epic sq-pbz04.2): ABox internalization + realisation + whole-ontology
// consistency — the opt-in `abox` feature.
//
// The TBox classifier (`Classifier::classify` / `classify_graph`) is instance-agnostic by
// contract. This module adds the ABox layer WITHOUT disturbing it: it runs the SAME
// normalize → CR1–CR6 saturation over the TBox axioms PLUS the ABox assertions internalized as
// safe-nominal axioms (`extract::extract` with `ExtractOpts { abox: true }` — the internalization
// lives in `extract.rs` where it reuses the restriction/intersection `decode` machinery), then
// READS OFF the saturation:
//
//   {a} ⊑ C   (C a NAMED class, incl. owl:Thing)  ⇒  `a rdf:type C`
//   {a} ⊑ {b} (a derived nominal subsumption)      ⇒  `a owl:sameAs b`
//   ∃r.Self ∈ S({a})  (owl:hasSelf, sq-pbz04.2.6)  ⇒  `a r a`   (the CR-Self self-loop readoff)
//   (the CONVERSE — an asserted/derived same-nominal self-link `a r a` ⇒ `∃r.Self ∈ S({a})` —
//   is the [FABLE-5] sq-8zqwb saturation rule CRs3 in `classify.rs`, so `∃r.Self ⊑ D` typings
//   realise off a raw self-loop; the WG `New-Feature-SelfRestriction-002` converse shape)
//   {a} ⊑ ⊥   OR  ⊤ ⊑ ⊥                            ⇒  a whole-ontology `inconsistent` verdict
//
// [OPUS-4.8] sq-pbz04.2.8 (E4) — three more ABox mechanisms read off the SAME saturation, all
// soundness-over-completeness (each verdict holds in EVERY model of the input):
//   * `owl:hasKey(C, keys)` — two DISTINCT named (IRI) individuals BOTH derivably in `C`, sharing a
//     derivable value on EVERY key property, are the SAME (`owl:sameAs`). Sound: a shared ASSERTED/
//     DERIVED value on every key witnesses the OWL 2 key-satisfaction antecedent, so `a = b` in every
//     model. Firing on a PARTIAL match is impossible BY CONSTRUCTION (the per-property shared-value
//     sets must ALL be non-empty AND pairwise-intersecting). Object keys match on a shared nominal
//     successor; data keys on a shared literal TERM (identical terms ⇒ identical values — sound).
//   * negative property assertions (`owl:NegativePropertyAssertion`) — a clash (whole-ontology
//     inconsistency) iff the corresponding POSITIVE assertion is asserted or DERIVED. (Object: a
//     nominal successor of `{a}` via `p`. Data: an asserted `a p "v"` triple — data positives are
//     not internalized, an honest incompleteness, never an unsound missed clash.)
//   * `owl:differentFrom` — read off ONLY from a DERIVED nominal clash (`{a} ⊓ {b} ⊑ ⊥`, e.g. `a : A`,
//     `b : B`, `A ⊓ B ⊑ ⊥`) or the SYMMETRIC closure of an asserted inequality — never fabricated.
//     An asserted-or-derived inequality that coincides with a derived `owl:sameAs` is a whole-ontology
//     inconsistency (a proof that `a = b` AND `a ≠ b`).
//
// SOUNDNESS (the load-bearing invariant — soundness over completeness). The saturation maintains
// `D ∈ S(C) ⟹ T ⊨ C ⊑ D`. A nominal `{a}` denotes the singleton `{a^I}`, so:
//   * `C ∈ S({a})` with C named ⟹ `T ⊨ {a} ⊑ C` ⟹ `a^I ∈ C^I` — the typing holds in EVERY model.
//   * `{b} ∈ S({a})` ⟹ `T ⊨ {a} ⊑ {b}` ⟹ `a^I ∈ {b^I}` ⟹ `a^I = b^I` — `owl:sameAs` holds.
//   * `⊥ ∈ S({a})` ⟹ `{a}` is empty, but `a` witnesses it non-empty — a contradiction: the whole
//     ontology has no model. `⊥ ∈ S(⊤)` is the global `⊤ ⊑ ⊥` case.
// The CR6 reachability side-condition is untouched (this module only READS the saturation the
// existing `classify::saturate` produced). Unsupported assertion / key / NPA shapes stay counted
// skips (`Report::skipped_assertions`, fail-closed) — never a guessed typing/equality/clash.
//
// [SONNET-4.6] sq-vkq9u — the `abox` × `cdomain` seam. With BOTH features on, a
// `DataPropertyAssertion` whose literal lies in the exact numeric tier is no longer a skip:
// `extract::decode_abox` internalizes `a q 5` as `{a} ⊑ ∃q.{5}` over the point range CR9 mints
// (`{5}` is the SAME concept a TBox `DataHasValue 5` / `DataOneOf 5` / faceted `[5, 5]` resolves
// to), so CR8 containment carries an asserted VALUE into the TBox's data-range obligations and the
// typings / inconsistencies below read off it with no change to THIS module. Sound because
// `a q v` asserts `(a^I, v) ∈ q^I` and `v ∈ {v}^D`. The NPA/key readoffs are UNAFFECTED: both
// still match data positives against ASSERTED triples (the honest incompleteness noted above),
// not against the rescued existentials.

use crate::extract::{self, AboxNpa, ExtractOpts};
use crate::normal::{Concept, Names, Normal, BOTTOM, TOP};
use crate::{classify, ClassHierarchy, Report};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id, TermParts, NO_ID};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const OWL_THING: &str = "http://www.w3.org/2002/07/owl#Thing";
const OWL_SAME_AS: &str = "http://www.w3.org/2002/07/owl#sameAs";
const OWL_DIFFERENT_FROM: &str = "http://www.w3.org/2002/07/owl#differentFrom";

/// A list of `(individual dict id, class-or-individual dict id)` readoff rows.
type Pairs = Vec<(Id, Id)>;

fn lookup(dict: &Dict, iri: &str) -> Id {
    use oxrdf::{NamedNode, Term as OTerm};
    dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(iri.to_string())))
}

/// The result of ABox-aware classification (the `abox` feature): the TBox subsumption
/// [`ClassHierarchy`] PLUS the realised instance facts and the whole-ontology consistency verdict.
///
/// Every emitted typing / `owl:sameAs` holds in EVERY model of the input (soundness over
/// completeness); when the ontology is [inconsistent](Self::is_inconsistent) the realised facts
/// are EMPTY (an inconsistent ontology entails everything — the honest surface is the verdict
/// alone, not a flood of derived rows).
pub struct Realization {
    hierarchy: ClassHierarchy,
    types: Vec<(Id, Id)>,
    same_as: Vec<(Id, Id)>,
    /// [OPUS-4.8] sq-pbz04.2.6: `(individual, object-property)` pairs for each derived
    /// `owl:hasSelf` self-loop `a r a` (`∃r.Self ∈ S({a})`), sorted + deduplicated.
    self_loops: Vec<(Id, Id)>,
    /// [OPUS-4.8] sq-pbz04.2.8: derived `(a, b)` `owl:differentFrom` inequalities — the symmetric
    /// closure of asserted `owl:differentFrom` PLUS every derived nominal clash (`{a} ⊓ {b} ⊑ ⊥`),
    /// sorted + deduplicated. EMPTY when the ontology is inconsistent.
    different_from: Vec<(Id, Id)>,
}

impl Realization {
    /// The TBox class-subsumption lattice (unchanged by the ABox layer — nominals project out).
    pub fn class_hierarchy(&self) -> &ClassHierarchy {
        &self.hierarchy
    }

    /// Realised `(individual dict id, class dict id)` type assertions — the complete derived
    /// typing of every asserted individual (including `owl:Thing` when it is interned), sorted +
    /// deduplicated. EMPTY when the ontology is inconsistent.
    pub fn type_assertions(&self) -> &[(Id, Id)] {
        &self.types
    }

    /// Realised `(individual, individual)` `owl:sameAs` pairs derived from nominal equality,
    /// sorted + deduplicated. EMPTY when the ontology is inconsistent.
    pub fn same_as(&self) -> &[(Id, Id)] {
        &self.same_as
    }

    /// [OPUS-4.8] sq-pbz04.2.6: realised `owl:hasSelf` self-loops as `(individual, object-property)`
    /// pairs — each denotes the derived property assertion `a r a` from `∃r.Self ∈ S({a})` (i.e.
    /// `{a} ⊑ ∃r.Self`; the OWL WG `New-Feature-SelfRestriction-001` "Peter likes Peter" readoff).
    /// Sorted + deduplicated; EMPTY when the ontology is inconsistent. SOUND because `{a}` is the
    /// singleton `{a^I}`, so `{a} ⊑ ∃r.Self` forces `(a^I, a^I) ∈ r^I`.
    pub fn self_assertions(&self) -> &[(Id, Id)] {
        &self.self_loops
    }

    /// [OPUS-4.8] sq-pbz04.2.8: realised `owl:differentFrom` inequalities as `(a, b)` pairs — the
    /// symmetric closure of asserted `owl:differentFrom` plus every derived nominal clash
    /// (`{a} ⊓ {b} ⊑ ⊥`, e.g. two individuals in provably-disjoint classes). Sorted + deduplicated;
    /// EMPTY when the ontology is inconsistent. Every pair is SOUND (`a^I ≠ b^I` in every model);
    /// missing inequalities are honest incompleteness, never an unsound derivation.
    pub fn different_from(&self) -> &[(Id, Id)] {
        &self.different_from
    }

    /// Whether the WHOLE ontology is inconsistent: `{a} ⊑ ⊥` for an asserted individual, a global
    /// `⊤ ⊑ ⊥`, a [OPUS-4.8] sq-pbz04.2.8 negative property assertion whose positive is
    /// asserted/derived, or a proof of both `a = b` (`owl:sameAs`) and `a ≠ b` (`owl:differentFrom`).
    /// See [`ClassHierarchy::is_inconsistent`].
    pub fn is_inconsistent(&self) -> bool {
        self.hierarchy.is_inconsistent()
    }

    /// The extraction/classification [`Report`] — `skipped_assertions` is the count of assertions
    /// DEFERRED (fail-closed): a non-EL class expression, a structural-bnode object, or a
    /// data-property value with no minted point range (every literal without `cdomain`; a
    /// non-exact-numeric one with it — see [SONNET-4.6] sq-vkq9u in the module docs).
    /// `unsatisfiable_classes` is the TBox named-class clash count.
    pub fn report(&self) -> Report {
        self.hierarchy.report()
    }
}

/// Classifies the EL+⊥ TBox in `triples` AND internalizes its ABox (`ClassAssertion` /
/// `ObjectPropertyAssertion`) as safe-nominal axioms, returning the realised typings,
/// `owl:sameAs` pairs, and whole-ontology consistency verdict. Does NOT mutate the graph; use
/// [`realize_graph`] to materialize the readoff as triples. Single-threaded.
///
/// ```
/// use sparq_core::Graph;
/// use sparq_reason_el::realize;
///
/// // Two disjoint ClassAssertions on one individual ⇒ the ontology is inconsistent
/// // (the OWL WG `DisjointClasses-002` shape — an ABox clash the TBox classifier cannot see).
/// let ttl = r#"
///   @prefix : <http://ex/> .
///   @prefix owl: <http://www.w3.org/2002/07/owl#> .
///   :Boy owl:disjointWith :Girl .
///   :Stewie a :Boy , :Girl .
/// "#;
/// let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").unwrap();
/// let r = realize(&dict, &triples);
/// assert!(r.is_inconsistent());
/// ```
pub fn realize(dict: &Dict, triples: &[[Id; 3]]) -> Realization {
    let ex = extract::extract(dict, triples, ExtractOpts { abox: true });
    let sat = crate::saturate_extracted(&ex);
    let cls = classify::classify(&sat, &ex.names);
    let mut hierarchy = crate::hierarchy_from(cls, &ex.names, ex.report);

    // Whole-ontology inconsistency: an asserted individual forced into ⊥, or a global ⊤ ⊑ ⊥.
    let base_inconsistent = sat.s[TOP as usize].contains(&BOTTOM)
        || ex
            .names
            .nominals()
            .any(|(_, c)| sat.s[c as usize].contains(&BOTTOM));

    // [OPUS-4.8] sq-pbz04.2.8 (E4): a negative property assertion whose positive is asserted/derived
    // is a whole-ontology clash. Checked only when the base ontology is otherwise consistent (an
    // already-inconsistent ontology is inconsistent regardless).
    let mut inconsistent = base_inconsistent || npa_clashes(&sat, &ex, triples);

    let (types, same_as, self_loops, different_from) = if inconsistent {
        // An inconsistent ontology entails everything — surface ONLY the verdict, not a flood.
        (Vec::new(), Vec::new(), Vec::new(), Vec::new())
    } else {
        let (types, mut same_as, self_loops) = readoff(dict, &sat, &ex.names);
        // [OPUS-4.8] sq-pbz04.2.8: key-induced merges add to the CR6 `owl:sameAs` readoff.
        same_as.extend(key_same_as(dict, &sat, &ex, triples));
        same_as.sort_unstable();
        same_as.dedup();
        // Derived `owl:differentFrom` (asserted symmetric closure + derived nominal clashes).
        let different_from = derive_different_from(dict, &sat, &ex);
        // A proof of both `a = b` and `a ≠ b` is a whole-ontology inconsistency — surface the
        // verdict, NOT the contradictory rows.
        let same_set: FxHashSet<(Id, Id)> = same_as.iter().copied().collect();
        if different_from.iter().any(|p| same_set.contains(p)) {
            inconsistent = true;
            (Vec::new(), Vec::new(), Vec::new(), Vec::new())
        } else {
            (types, same_as, self_loops, different_from)
        }
    };
    hierarchy.inconsistent = inconsistent;
    Realization {
        hierarchy,
        types,
        same_as,
        self_loops,
        different_from,
    }
}

/// [OPUS-4.8] sq-pbz04.2.8: whether any negative property assertion clashes with an asserted/derived
/// positive. An object NPA `¬(a p b)` clashes iff `{b}` is a nominal successor of `{a}` via `p` (the
/// R-link, which the CR3 internalization of the positive `a p b` — asserted OR derived — populates),
/// OR the positive triple `[a p b]` is directly asserted. A data NPA `¬(a p "v")` clashes iff the
/// positive triple `[a p "v"]` is asserted (data positives are not internalized — an honest
/// incompleteness for a derived data positive, never an unsound missed clash). SOUND: an entailed
/// positive contradicts the entailed negative, so no model exists.
fn npa_clashes(sat: &classify::Saturation, ex: &extract::Extracted, triples: &[[Id; 3]]) -> bool {
    ex.abox_extras.npas.iter().any(|npa| match *npa {
        AboxNpa::Object {
            source_dict,
            prop_dict,
            target_dict,
            source_c,
            role,
            target_c,
        } => {
            triples.contains(&[source_dict, prop_dict, target_dict])
                || sat
                    .r_succ
                    .get(&role)
                    .and_then(|by_x| by_x.get(&source_c))
                    .is_some_and(|succ| succ.contains(&target_c))
        }
        AboxNpa::Data {
            source_dict,
            prop_dict,
            value_dict,
        } => triples.contains(&[source_dict, prop_dict, value_dict]),
    })
}

/// [OPUS-4.8] sq-pbz04.2.8: whether a dict id is a NAMED (IRI) individual — keys apply only to named
/// individuals (OWL 2 Direct Semantics) and the derived-`differentFrom` readoff stays on named
/// individuals (blank-node inequalities are noise), so both filter on this.
fn is_named_iri(dict: &Dict, id: Id) -> bool {
    !sparq_core::dict::is_inline(id) && matches!(dict.term_parts(id), TermParts::Iri { .. })
}

/// [OPUS-4.8] sq-pbz04.2.8: the `owl:hasKey`-induced `owl:sameAs` merges. For each key, two DISTINCT
/// named (IRI) individuals BOTH derivably in the key class that share a value on EVERY key property
/// are merged. Object-key values are the key role's NAMED nominal successors in the saturation
/// (`{b}` ∈ `R(p)[{a}]`, asserted or derived); data-key values are the asserted literal objects
/// (`[a p "v"]`). SOUNDNESS: a shared value on every key property is exactly the OWL 2 key antecedent,
/// so `a = b` in every model; a PARTIAL match cannot fire because the per-property value sets must
/// ALL be non-empty AND pairwise-intersect. Emits both `(a, b)` and `(b, a)` (equality is symmetric).
fn key_same_as(
    dict: &Dict,
    sat: &classify::Saturation,
    ex: &extract::Extracted,
    triples: &[[Id; 3]],
) -> Vec<(Id, Id)> {
    let mut out: Vec<(Id, Id)> = Vec::new();
    if ex.abox_extras.keys.is_empty() {
        return out;
    }
    // Reverse map: nominal concept -> its individual dict id.
    let nom_to_dict: FxHashMap<Concept, Id> = ex.names.nominals().map(|(id, c)| (c, id)).collect();
    // Asserted literal values, indexed by (property dict id, subject dict id), for data keys.
    let key_props: FxHashSet<Id> = ex
        .abox_extras
        .keys
        .iter()
        .flat_map(|k| k.props.iter().map(|&(_, p)| p))
        .collect();
    let mut data_vals: FxHashMap<(Id, Id), FxHashSet<Id>> = FxHashMap::default();
    if !key_props.is_empty() {
        for &[s, p, o] in triples {
            if key_props.contains(&p) && is_literal_id(dict, o) {
                data_vals.entry((p, s)).or_default().insert(o);
            }
        }
    }

    for key in &ex.abox_extras.keys {
        // Candidate members: NAMED individuals with the key class in their S-set, each carrying its
        // per-property value set. Skip any member lacking a value on SOME key property — an
        // incomplete key does not constrain it (this is the guard that makes a PARTIAL match unable
        // to fire).
        let mut cand: Vec<(Id, Vec<FxHashSet<Id>>)> = Vec::new();
        for (m_id, m_c) in ex.names.nominals() {
            if !is_named_iri(dict, m_id) || !sat.s[m_c as usize].contains(&key.class) {
                continue;
            }
            let mut sig: Vec<FxHashSet<Id>> = Vec::with_capacity(key.props.len());
            let mut complete = true;
            for &(role, prop) in &key.props {
                let mut vs: FxHashSet<Id> = FxHashSet::default();
                // Object values: named nominal successors via the key role.
                if let Some(succ) = sat.r_succ.get(&role).and_then(|by_x| by_x.get(&m_c)) {
                    for &vc in succ {
                        if let Some(&vd) = nom_to_dict.get(&vc) {
                            if is_named_iri(dict, vd) {
                                vs.insert(vd);
                            }
                        }
                    }
                }
                // Data values: asserted literal objects.
                if let Some(lits) = data_vals.get(&(prop, m_id)) {
                    vs.extend(lits.iter().copied());
                }
                if vs.is_empty() {
                    complete = false;
                    break;
                }
                sig.push(vs);
            }
            if complete {
                cand.push((m_id, sig));
            }
        }
        // Merge every pair sharing a value on EVERY key property.
        for i in 0..cand.len() {
            for j in (i + 1)..cand.len() {
                let matches = cand[i]
                    .1
                    .iter()
                    .zip(&cand[j].1)
                    .all(|(a, b)| !a.is_disjoint(b));
                if matches {
                    out.push((cand[i].0, cand[j].0));
                    out.push((cand[j].0, cand[i].0));
                }
            }
        }
    }
    out
}

/// [OPUS-4.8] sq-pbz04.2.8: whether a dict id denotes a LITERAL (an inline integer or a stored
/// `Lit`) — the object shape of a data-property value.
fn is_literal_id(dict: &Dict, id: Id) -> bool {
    sparq_core::dict::is_inline(id) || matches!(dict.term_parts(id), TermParts::Lit { .. })
}

/// [OPUS-4.8] sq-pbz04.2.8: the derived `owl:differentFrom` pairs — the SYMMETRIC closure of asserted
/// `owl:differentFrom`, PLUS every derived nominal clash. A clash `{a} ⊓ {b} ⊑ ⊥` is witnessed by a
/// disjointness axiom `c1 ⊓ c2 ⊑ ⊥` with `c1 ∈ S({a})` and `c2 ∈ S({b})`: if `a = b` then
/// `a^I ∈ c1^I ∩ c2^I = ∅`, impossible, so `a ≠ b` in every model (SOUND). Named (IRI) individuals
/// only. Both directions are emitted; the caller sorts + deduplicates.
fn derive_different_from(
    dict: &Dict,
    sat: &classify::Saturation,
    ex: &extract::Extracted,
) -> Vec<(Id, Id)> {
    let mut out: Vec<(Id, Id)> = Vec::new();
    // Asserted `owl:differentFrom`, symmetric closure.
    for &(a, b) in &ex.abox_extras.different_from {
        out.push((a, b));
        out.push((b, a));
    }
    // Derived nominal clashes from binary disjointness axioms `c1 ⊓ c2 ⊑ ⊥`.
    let has_disjoint = ex
        .axioms
        .iter()
        .any(|ax| matches!(ax, Normal::AndSub(_, _, BOTTOM)));
    if has_disjoint {
        // Named-individual nominals only.
        let named: Vec<(Id, Concept)> = ex
            .names
            .nominals()
            .filter(|&(id, _)| is_named_iri(dict, id))
            .collect();
        for ax in &ex.axioms {
            if let Normal::AndSub(c1, c2, BOTTOM) = *ax {
                let holds = |c: Concept| -> Vec<Id> {
                    named
                        .iter()
                        .filter(|&&(_, nc)| sat.s[nc as usize].contains(&c))
                        .map(|&(id, _)| id)
                        .collect()
                };
                let a1 = holds(c1);
                let a2 = holds(c2);
                for &a in &a1 {
                    for &b in &a2 {
                        if a != b {
                            out.push((a, b));
                            out.push((b, a));
                        }
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Reads the realised type assertions + `owl:sameAs` pairs off a saturated (CONSISTENT)
/// classification. `owl:Thing` typing is emitted per individual only when `owl:Thing` is interned
/// (it is a trivially-true type; [`realize_graph`] interns it so it is always emitted there).
fn readoff(dict: &Dict, sat: &classify::Saturation, names: &Names) -> (Pairs, Pairs, Pairs) {
    let owl_thing = lookup(dict, OWL_THING);
    // concept → individual dict id, for the sameAs reverse mapping.
    let rev: FxHashMap<Concept, Id> = names.nominals().map(|(id, c)| (c, id)).collect();
    let mut types: Pairs = Vec::new();
    let mut same_as: Pairs = Vec::new();
    let mut self_loops: Pairs = Vec::new();
    for (ind, nc) in names.nominals() {
        if owl_thing != NO_ID {
            types.push((ind, owl_thing));
        }
        for &d in &sat.s[nc as usize] {
            if d == nc || d == TOP || d == BOTTOM {
                continue;
            }
            if names.is_nominal(d) {
                if let Some(&other) = rev.get(&d) {
                    if other != ind {
                        // `{other} ∈ S({ind})` ⟹ `ind = other`; equality is symmetric, so both
                        // directions are sound entailments.
                        same_as.push((ind, other));
                        same_as.push((other, ind));
                    }
                }
            } else if let Some(r) = names.self_role(d) {
                // [OPUS-4.8] sq-pbz04.2.6: `∃r.Self ∈ S({ind})` ⟹ `{ind} ⊑ ∃r.Self`; since
                // `{ind}` is the singleton `{ind^I}`, the r-self-loop is at `ind` itself, so
                // `ind r ind` holds in EVERY model. (The role is always a named property.)
                if let Some(rd) = names.role_dict_of(r) {
                    self_loops.push((ind, rd));
                }
            } else if let Some(cd) = names.dict_of(d) {
                types.push((ind, cd));
            }
        }
    }
    types.sort_unstable();
    types.dedup();
    same_as.sort_unstable();
    same_as.dedup();
    self_loops.sort_unstable();
    self_loops.dedup();
    (types, same_as, self_loops)
}

/// The report of a materializing [`realize_graph`] run: the base [`Report`] (skipped assertions,
/// unsatisfiable classes), the whole-ontology `inconsistent` verdict, and how many NEW readoff
/// rows were appended. When `inconsistent`, NO realisation rows are emitted (honest — the verdict
/// is the surface, not an everything-entailed flood).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AboxReport {
    /// The extraction/classification report (`skipped_assertions`, `unsatisfiable_classes`, …).
    pub report: Report,
    /// Whole-ontology inconsistency (`{a} ⊑ ⊥` or `⊤ ⊑ ⊥`).
    pub inconsistent: bool,
    /// NEW `a rdf:type C` triples appended.
    pub emitted_type_assertions: usize,
    /// NEW `a owl:sameAs b` triples appended.
    pub emitted_same_as: usize,
    /// [OPUS-4.8] sq-pbz04.2.6: NEW `a r a` `owl:hasSelf` self-loop triples appended.
    pub emitted_self_assertions: usize,
    /// [OPUS-4.8] sq-pbz04.2.8: NEW `a owl:differentFrom b` triples appended.
    pub emitted_different_from: usize,
}

/// Classifies + realises the ABox and MATERIALIZES the readoff IN PLACE: appends every derived
/// `(a rdf:type C)` (including `a rdf:type owl:Thing`, which is interned so it is always emitted),
/// `(a owl:sameAs b)` (CR6 nominal equality + [OPUS-4.8] sq-pbz04.2.8 `owl:hasKey` merges),
/// `(a r a)` (sq-pbz04.2.6 `owl:hasSelf` self-loop) and `(a owl:differentFrom b)` (sq-pbz04.2.8)
/// not already present, interning `rdf:type` / `owl:sameAs` / `owl:differentFrom` / `owl:Thing` as
/// needed. Whole-ontology inconsistency is surfaced via [`AboxReport::inconsistent`] (NOT as a
/// triple — the caller decides how to flag it), matching the TBox `classify_graph` convention of
/// reporting clashes rather than emitting them. When inconsistent, NO rows are appended.
///
/// Idempotent: a second call adds nothing. The emitted edges are ordinary triples queryable by
/// plain BGP eval — the same `(Dict, triples)` seam the RL `scm-*` / EL `classify_graph` output
/// uses.
pub fn realize_graph(dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> AboxReport {
    // Intern the readoff vocabulary FIRST so `realize` sees `owl:Thing` and emits its typing for
    // every individual (a trivially-true type the WG `*-Ontology-001` conclusions assert).
    let rdf_type = dict.intern_iri(RDF_TYPE);
    let _owl_thing = dict.intern_iri(OWL_THING);
    let owl_same_as = dict.intern_iri(OWL_SAME_AS);
    let owl_different_from = dict.intern_iri(OWL_DIFFERENT_FROM);

    let r = realize(dict, triples);
    let inconsistent = r.is_inconsistent();
    let report = r.report();

    let mut present: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    let mut emitted_type_assertions = 0usize;
    let mut emitted_same_as = 0usize;
    let mut emitted_self_assertions = 0usize;
    let mut emitted_different_from = 0usize;
    if !inconsistent {
        for &(ind, cls) in r.type_assertions() {
            let t = [ind, rdf_type, cls];
            if present.insert(t) {
                triples.push(t);
                emitted_type_assertions += 1;
            }
        }
        for &(a, b) in r.same_as() {
            let t = [a, owl_same_as, b];
            if present.insert(t) {
                triples.push(t);
                emitted_same_as += 1;
            }
        }
        // [OPUS-4.8] sq-pbz04.2.6: `owl:hasSelf` self-loops `a r a`. The role predicate `r` is
        // already interned (it occurred as an `owl:onProperty` object in the input), so no new
        // vocabulary is minted; hasSelf-free inputs read off no self-loops (byte-identical).
        for &(ind, role) in r.self_assertions() {
            let t = [ind, role, ind];
            if present.insert(t) {
                triples.push(t);
                emitted_self_assertions += 1;
            }
        }
        // [OPUS-4.8] sq-pbz04.2.8: `owl:differentFrom` inequalities.
        for &(a, b) in r.different_from() {
            let t = [a, owl_different_from, b];
            if present.insert(t) {
                triples.push(t);
                emitted_different_from += 1;
            }
        }
    }
    AboxReport {
        report,
        inconsistent,
        emitted_type_assertions,
        emitted_same_as,
        emitted_self_assertions,
        emitted_different_from,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Classifier;
    use oxrdf::{NamedNode, Term as OTerm};
    use sparq_core::Graph;

    fn iri(dict: &Dict, s: &str) -> Id {
        dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(s.to_string())))
    }

    fn parse(ttl: &str) -> (Dict, Vec<[Id; 3]>) {
        Graph::parse_to_triples(ttl, "turtle").expect("parse")
    }

    const PRE: &str = r#"
        @prefix : <http://ex/> .
        @prefix owl: <http://www.w3.org/2002/07/owl#> .
        @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    "#;

    // --- WebOnt-Ontology-001: rdf:type owl:Thing (+ equivalentClass) realisation readoff. -----
    #[test]
    fn webont_ontology_001_thing_typing_and_equivalence() {
        // Car ≡ Automobile; car : Car, owl:Thing ; auto : Automobile, owl:Thing.
        let ttl = format!(
            "{PRE}
             :Car owl:equivalentClass :Automobile .
             :car a :Car , owl:Thing .
             :auto a :Automobile , owl:Thing ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (car, auto) = (iri(&dict, "http://ex/car"), iri(&dict, "http://ex/auto"));
        let (thing, cls_car, cls_auto) = (
            iri(&dict, "http://www.w3.org/2002/07/owl#Thing"),
            iri(&dict, "http://ex/Car"),
            iri(&dict, "http://ex/Automobile"),
        );
        let ty = r.type_assertions();
        // The conclusion: both individuals are instances of owl:Thing ...
        assert!(ty.contains(&(car, thing)), "car rdf:type owl:Thing");
        assert!(ty.contains(&(auto, thing)), "auto rdf:type owl:Thing");
        // ... and Car ≡ Automobile makes each an instance of BOTH named classes.
        assert!(ty.contains(&(car, cls_car)) && ty.contains(&(car, cls_auto)));
        assert!(ty.contains(&(auto, cls_car)) && ty.contains(&(auto, cls_auto)));
    }

    // --- DisjointClasses-002: an ABox-driven inconsistency (disjoint dual ClassAssertion). -----
    #[test]
    fn disjoint_classes_002_abox_inconsistency() {
        let ttl = format!(
            "{PRE}
             :Boy owl:disjointWith :Girl .
             :Stewie a :Boy , :Girl ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.is_inconsistent(), "Stewie ∈ Boy ⊓ Girl ⊑ ⊥ ⇒ inconsistent");
        assert!(r.type_assertions().is_empty(), "no realisation for an inconsistent ontology");
        // The load-bearing invariant: Boy/Girl are each individually SATISFIABLE — the clash is
        // ABox-only, so the TBox classifier (byte-identical here) flags NO unsatisfiable class.
        let h = Classifier::classify(&dict, &triples);
        assert!(h.unsatisfiable_classes().is_empty());
        assert!(!h.is_inconsistent(), "TBox path never decides whole-ontology consistency");
    }

    // --- New-Feature-BottomObjectProperty-001: an ABox instance of ∃⊥.⊤ ⊑ ⊥. -----------------
    #[test]
    fn bottom_object_property_001_empty_role_clash() {
        // i : [ ∃owl:bottomObjectProperty.owl:Thing ] — owl:bottomObjectProperty is empty, so
        // the existential is unsatisfiable and the asserted instance forces inconsistency.
        let ttl = format!(
            "{PRE}
             :i a [ a owl:Restriction ;
                    owl:onProperty owl:bottomObjectProperty ;
                    owl:someValuesFrom owl:Thing ] ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.is_inconsistent(), "∃⊥.⊤ is empty ⇒ {{i}} ⊑ ⊥ ⇒ inconsistent");
    }

    // --- WebOnt-Restriction-001: an ABox instance of ∃op.owl:Nothing (CR5 clash). -------------
    #[test]
    fn webont_restriction_001_existential_bottom_filler() {
        // a, b : [ ∃op.owl:Nothing ] — a someValuesFrom into ⊥ is empty (CR5), so the asserted
        // instances force inconsistency, yet op / the anonymous class name no NAMED unsat class.
        let ttl = format!(
            "{PRE}
             :op a owl:ObjectProperty .
             :a a [ a owl:Restriction ; owl:onProperty :op ; owl:someValuesFrom owl:Nothing ] .
             :b a [ a owl:Restriction ; owl:onProperty :op ; owl:someValuesFrom owl:Nothing ] ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.is_inconsistent(), "∃op.⊥ ⊑ ⊥ (CR5) ⇒ asserted instances ⇒ inconsistent");
        let h = Classifier::classify(&dict, &triples);
        assert!(h.unsatisfiable_classes().is_empty(), "no NAMED class is unsatisfiable");
    }

    // --- WebOnt-Thing-003: a global ⊤ ⊑ ⊥ inconsistency, no individuals needed. ----------------
    #[test]
    fn webont_thing_003_global_top_bottom() {
        let ttl = format!("{PRE} owl:Thing owl:equivalentClass owl:Nothing .");
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.is_inconsistent(), "⊤ ≡ ⊥ ⇒ ⊥ ∈ S(⊤) ⇒ global inconsistency");
        // The TBox classifier, by its documented contract, decides only named-class satisfiability
        // and flags nothing (⊤/⊥ are excluded from that surface).
        let h = Classifier::classify(&dict, &triples);
        assert!(!h.is_inconsistent());
    }

    // --- ObjectPropertyAssertion internalization: {a} ⊑ ∃p.{b}, threaded through a TBox axiom. -
    #[test]
    fn object_property_assertion_types_through_existential() {
        // p some Parent ⊑ Fond ; alice p bob, bob : Parent  ⊨  alice : Fond  (CR4 through {bob}).
        let ttl = format!(
            "{PRE}
             [ a owl:Restriction ; owl:onProperty :p ; owl:someValuesFrom :Parent ]
                 rdfs:subClassOf :Fond .
             :bob a :Parent .
             :alice :p :bob ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (alice, fond) = (iri(&dict, "http://ex/alice"), iri(&dict, "http://ex/Fond"));
        assert!(
            r.type_assertions().contains(&(alice, fond)),
            "alice ∈ ∃p.Parent ⊑ Fond via the asserted p-edge to bob : Parent"
        );
    }

    // --- DataPropertyAssertion: a counted skip, or the `cdomain` point-range rescue. -----------
    #[test]
    fn data_property_assertion_is_a_counted_skip() {
        let ttl = format!(
            "{PRE}
             :alice a :Person .
             :alice :age 30 ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        // WITHOUT `cdomain` there is no value tower to mint `{30}` on, so the assertion stays a
        // fail-closed counted skip. WITH it, [SONNET-4.6] sq-vkq9u internalizes `{alice} ⊑
        // ∃age.{30}` and nothing is skipped — the rescue's own oracles live in
        // tests/abox_cdomain.rs (that feature CONJUNCTION is its own feature-matrix leg).
        #[cfg(not(feature = "cdomain"))]
        assert_eq!(r.report().skipped_assertions, 1, "the data-property assertion is skipped");
        #[cfg(feature = "cdomain")]
        assert_eq!(r.report().skipped_assertions, 0, "sq-vkq9u rescues `:age 30` as ∃age.{{30}}");
        // The class assertion still classifies the individual, in either feature state.
        let (alice, person) = (iri(&dict, "http://ex/alice"), iri(&dict, "http://ex/Person"));
        assert!(r.type_assertions().contains(&(alice, person)));
    }

    // --- A non-EL ClassAssertion class expression stays a counted skip (fail-closed). ----------
    #[test]
    fn non_el_class_assertion_is_skipped() {
        let ttl = format!(
            "{PRE}
             :x a [ owl:unionOf ( :A :B ) ] .
             :y a :C ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.report().skipped_assertions >= 1, "the unionOf class assertion is skipped");
        let (y, c) = (iri(&dict, "http://ex/y"), iri(&dict, "http://ex/C"));
        assert!(r.type_assertions().contains(&(y, c)), "the EL class assertion still classifies");
    }

    // --- ObjectPropertyAssertion with a structural bnode object is a counted skip. -------------
    // [OPUS-4.8] sq-pbz04.2.5: previously the case was sound-and-fail-closed but untallied;
    // this test pins that it is NOW counted in `skipped_assertions` rather than silently dropped.
    #[test]
    fn object_property_assertion_structural_bnode_object_is_counted_skip() {
        // :alice :p [ a owl:Restriction ; owl:onProperty :q ; owl:someValuesFrom :D ] .
        // The object blank node is structural (typed owl:Restriction, has onProperty/svf edges).
        // The subject is a plain IRI individual, so the assertion is an OPA — but the object
        // cannot be interned as a safe nominal. Fail-closed: counted as a skip, never guessed.
        let ttl = format!(
            "{PRE}
             :alice :p [ a owl:Restriction ;
                         owl:onProperty :q ;
                         owl:someValuesFrom :D ] .
             :alice a :Person ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert_eq!(
            r.report().skipped_assertions,
            1,
            "the OPA with a structural-bnode object must be counted as a skip"
        );
        // The ClassAssertion still classifies alice correctly.
        let (alice, person) = (iri(&dict, "http://ex/alice"), iri(&dict, "http://ex/Person"));
        assert!(
            r.type_assertions().contains(&(alice, person)),
            "alice rdf:type Person still realized"
        );
    }

    // --- realize_graph materializes the readoff and is idempotent. -----------------------------
    #[test]
    fn realize_graph_materializes_typings_and_is_idempotent() {
        let ttl = format!(
            "{PRE}
             :A rdfs:subClassOf :B .
             :i a :A ."
        );
        let (mut dict, mut triples) = parse(&ttl);
        let before = triples.len();
        let rep = realize_graph(&mut dict, &mut triples);
        assert!(!rep.inconsistent);
        assert!(rep.emitted_type_assertions >= 1, "at least i rdf:type A/B/owl:Thing emitted");
        assert!(triples.len() > before);
        let (i, a, b) = (
            iri(&dict, "http://ex/i"),
            iri(&dict, "http://ex/A"),
            iri(&dict, "http://ex/B"),
        );
        let rdf_type = iri(&dict, RDF_TYPE);
        assert!(triples.contains(&[i, rdf_type, a]), "i rdf:type A materialized");
        assert!(triples.contains(&[i, rdf_type, b]), "i rdf:type B (derived) materialized");
        let rep2 = realize_graph(&mut dict, &mut triples);
        assert_eq!(rep2.emitted_type_assertions, 0, "second call is idempotent");
        assert_eq!(rep2.emitted_same_as, 0);
    }

    // --- realize_graph emits NOTHING for an inconsistent ontology (verdict only). --------------
    #[test]
    fn realize_graph_emits_nothing_when_inconsistent() {
        let ttl = format!(
            "{PRE}
             :Boy owl:disjointWith :Girl .
             :Stewie a :Boy , :Girl ."
        );
        let (mut dict, mut triples) = parse(&ttl);
        let before = triples.len();
        let rep = realize_graph(&mut dict, &mut triples);
        assert!(rep.inconsistent);
        assert_eq!(rep.emitted_type_assertions, 0);
        assert_eq!(rep.emitted_same_as, 0);
        assert_eq!(triples.len(), before, "no rows appended for an inconsistent ontology");
    }

    // --- owl:sameAs readoff from a derived nominal subsumption (singleton oneOf ClassAssertion). -
    #[test]
    fn same_as_readoff_from_singleton_one_of_assertion() {
        // `a rdf:type { b }` (a singleton `owl:oneOf` enumeration) ⇒ `{a} ⊑ {b}` ⇒ a = b.
        let ttl = format!(
            "{PRE}
             :a a [ owl:oneOf ( :b ) ] ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (a, b) = (iri(&dict, "http://ex/a"), iri(&dict, "http://ex/b"));
        let sa = r.same_as();
        assert!(sa.contains(&(a, b)), "a owl:sameAs b");
        assert!(sa.contains(&(b, a)), "owl:sameAs is emitted symmetrically");
    }

    // --- New-Feature-SelfRestriction-001: `Peter a [hasSelf likes]` ⊨ `Peter likes Peter`. -----
    // [OPUS-4.8] sq-pbz04.2.6: the E1 realisation readoff of the CR-Self-1 self-loop.
    #[test]
    fn self_restriction_001_peter_likes_peter() {
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             :Peter a [ a owl:Restriction ;
                        owl:onProperty :likes ;
                        owl:hasSelf \"true\"^^xsd:boolean ] ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (peter, likes) = (iri(&dict, "http://ex/Peter"), iri(&dict, "http://ex/likes"));
        assert!(
            r.self_assertions().contains(&(peter, likes)),
            "{{Peter}} ⊑ ∃likes.Self ⊨ Peter likes Peter (CR-Self-1 self-loop readoff)"
        );
        // Sound + fail-closed: the ONLY realised self-loop is Peter's own.
        assert_eq!(r.self_assertions(), &[(peter, likes)]);
    }

    // --- realize_graph materializes the self-loop and is idempotent. ---------------------------
    #[test]
    fn self_restriction_001_realize_graph_materializes_self_loop() {
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             :Peter a [ a owl:Restriction ; owl:onProperty :likes ; owl:hasSelf \"true\"^^xsd:boolean ] ."
        );
        let (mut dict, mut triples) = parse(&ttl);
        let rep = realize_graph(&mut dict, &mut triples);
        assert!(!rep.inconsistent);
        assert_eq!(rep.emitted_self_assertions, 1, "Peter likes Peter emitted once");
        let (peter, likes) = (iri(&dict, "http://ex/Peter"), iri(&dict, "http://ex/likes"));
        assert!(triples.contains(&[peter, likes, peter]), "Peter likes Peter materialized");
        let rep2 = realize_graph(&mut dict, &mut triples);
        assert_eq!(rep2.emitted_self_assertions, 0, "second call is idempotent");
    }

    // --- The CR-Self-1 self-loop LINK threads a typing through CR4 (not just a readoff trick). --
    // ∃likes.Person ⊑ Sociable ; Peter a [hasSelf likes] , Person ⊨ Peter : Sociable, because
    // Peter's own likes-self-loop witnesses an r-successor (Peter) that is a Person.
    #[test]
    fn self_loop_threads_a_typing_through_cr4() {
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             [ a owl:Restriction ; owl:onProperty :likes ; owl:someValuesFrom :Person ]
                 rdfs:subClassOf :Sociable .
             :Peter a [ a owl:Restriction ; owl:onProperty :likes ; owl:hasSelf \"true\"^^xsd:boolean ] .
             :Peter a :Person ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (peter, sociable) = (iri(&dict, "http://ex/Peter"), iri(&dict, "http://ex/Sociable"));
        assert!(
            r.type_assertions().contains(&(peter, sociable)),
            "the likes-self-loop must let CR4 derive Peter : Sociable"
        );
    }

    // --- New-Feature-SelfRestriction-002 (converse): the nominal-reflexivity readoff. ----------
    // [FABLE-5] sq-8zqwb (EL wave-2): `Peter likes Peter` internalizes as the SAME-NOMINAL
    // self-link `({Peter},{Peter}) ∈ R(likes)`, which CRs3 reads off as `∃likes.Self ∈
    // S({Peter})` — so `∃likes.Self ⊑ SelfLover` fires via CR1 AND the self-loop realises.
    // GRADUATES the honest-boundary fixture sq-pbz04.2.6 pinned here (deferred from #1681).
    // SOUND because a nominal is a singleton: Peter's likes-successor inside {Peter} is Peter
    // himself, so `(Peter,Peter) ∈ likes` holds in every model — exactly `Peter ∈ ∃likes.Self`.
    #[test]
    fn self_restriction_002_converse_nominal_reflexivity_readoff() {
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             [ a owl:Restriction ; owl:onProperty :likes ; owl:hasSelf \"true\"^^xsd:boolean ]
                 rdfs:subClassOf :SelfLover .
             :Peter :likes :Peter ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent(), "a self-loving individual is consistent");
        let (peter, likes, self_lover) = (
            iri(&dict, "http://ex/Peter"),
            iri(&dict, "http://ex/likes"),
            iri(&dict, "http://ex/SelfLover"),
        );
        assert!(
            r.type_assertions().contains(&(peter, self_lover)),
            "Peter likes Peter ⊨ Peter ∈ ∃likes.Self ⊑ SelfLover (the CRs3 converse readoff)"
        );
        assert!(
            r.self_assertions().contains(&(peter, likes)),
            "∃likes.Self ∈ S({{Peter}}) also reads off the (already-asserted) self-loop"
        );
    }

    // --- The CRs3 nominal guard is load-bearing: a GENERAL self-link must NOT read off. --------
    // [FABLE-5] sq-8zqwb: `C ⊑ ∃likes.C` creates the NON-nominal link (C,C) ∈ R(likes), whose
    // invariant is only `C ⊑ ∃likes.C` — each member of C has SOME likes-successor in C, not
    // necessarily itself — so a member of C must NOT be typed into ∃likes.Self.
    #[test]
    fn general_class_self_link_is_not_nominal_reflexivity() {
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             [ a owl:Restriction ; owl:onProperty :likes ; owl:hasSelf \"true\"^^xsd:boolean ]
                 rdfs:subClassOf :SelfLover .
             :C rdfs:subClassOf
                 [ a owl:Restriction ; owl:onProperty :likes ; owl:someValuesFrom :C ] .
             :x a :C ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (x, self_lover) = (iri(&dict, "http://ex/x"), iri(&dict, "http://ex/SelfLover"));
        assert!(
            !r.type_assertions().contains(&(x, self_lover)),
            "a general (C,C) link is not local reflexivity — CRs3 must not fire off it"
        );
        assert!(r.self_assertions().is_empty(), "no fabricated self-loop for x");
    }

    // --- CRs3 fires on a DERIVED same-nominal self-link too (owl:hasValue, no asserted OPA). ---
    // [FABLE-5] sq-8zqwb: `{a} ⊑ ∃p.{a}` via `owl:hasValue :a` derives the self-link without any
    // asserted `a p a` triple — still local reflexivity on the singleton {a}, so `∃p.Self ⊑ D`
    // fires and the self-loop `a p a` reads off as a NEW realised assertion.
    #[test]
    fn derived_nominal_self_link_reads_off_self() {
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             [ a owl:Restriction ; owl:onProperty :p ; owl:hasSelf \"true\"^^xsd:boolean ]
                 rdfs:subClassOf :D .
             :a a [ a owl:Restriction ; owl:onProperty :p ; owl:hasValue :a ] ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (a, p, d) = (
            iri(&dict, "http://ex/a"),
            iri(&dict, "http://ex/p"),
            iri(&dict, "http://ex/D"),
        );
        assert!(
            r.type_assertions().contains(&(a, d)),
            "derived {{a}} ⊑ ∃p.{{a}} ⊨ a ∈ ∃p.Self ⊑ D"
        );
        assert!(
            r.self_assertions().contains(&(a, p)),
            "the derived self-link reads off the realised self-loop a p a"
        );
    }

    // ======================================================================================
    // [OPUS-4.8] sq-pbz04.2.8 — E4: owl:hasKey + negative property assertions + differentFrom.
    // ======================================================================================

    // --- New-Feature-Keys-001: a DATA-property key merges two individuals sharing a key value. ---
    #[test]
    fn keys_001_data_key_merges_individuals() {
        // HasKey(owl:Thing () (:hasSSN)); Peter/Peter_Griffin both hasSSN "123-45-6789" ⊨ sameAs.
        let ttl = format!(
            "{PRE}
             owl:Thing owl:hasKey ( :hasSSN ) .
             :Peter :hasSSN \"123-45-6789\" .
             :Peter_Griffin :hasSSN \"123-45-6789\" ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (peter, pg) = (iri(&dict, "http://ex/Peter"), iri(&dict, "http://ex/Peter_Griffin"));
        let sa = r.same_as();
        assert!(sa.contains(&(peter, pg)), "Peter owl:sameAs Peter_Griffin (owl:hasKey)");
        assert!(sa.contains(&(pg, peter)), "owl:sameAs is emitted symmetrically");
    }

    // --- New-Feature-Keys-003: a LOCALIZED key merges only members of the key class. ------------
    #[test]
    fn keys_003_localized_key_excludes_non_members() {
        // HasKey(GriffinFamilyMember () (:hasName)); Peter/Peter_Griffin are members with hasName
        // "Peter"; StPeter ALSO has hasName "Peter" but is NOT in the class ⊨ only Peter=Peter_Griffin.
        let ttl = format!(
            "{PRE}
             :GriffinFamilyMember owl:hasKey ( :hasName ) .
             :Peter a :GriffinFamilyMember ; :hasName \"Peter\" .
             :Peter_Griffin a :GriffinFamilyMember ; :hasName \"Peter\" .
             :StPeter :hasName \"Peter\" ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (peter, pg, stpeter) = (
            iri(&dict, "http://ex/Peter"),
            iri(&dict, "http://ex/Peter_Griffin"),
            iri(&dict, "http://ex/StPeter"),
        );
        let sa = r.same_as();
        assert!(sa.contains(&(peter, pg)), "the two class members merge");
        assert!(
            !sa.iter().any(|&(a, b)| a == stpeter || b == stpeter),
            "StPeter is not in GriffinFamilyMember, so the localized key never touches it"
        );
    }

    // --- New-Feature-Keys-002: a key collision against an asserted differentFrom ⇒ inconsistent. -
    #[test]
    fn keys_002_collision_versus_different_from_is_inconsistent() {
        let ttl = format!(
            "{PRE}
             owl:Thing owl:hasKey ( :hasSSN ) .
             :Peter :hasSSN \"123-45-6789\" .
             :Peter_Griffin :hasSSN \"123-45-6789\" .
             :Peter owl:differentFrom :Peter_Griffin ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(
            r.is_inconsistent(),
            "the key forces Peter=Peter_Griffin against an asserted differentFrom"
        );
        assert!(r.same_as().is_empty(), "no realisation rows for an inconsistent ontology");
        assert!(r.different_from().is_empty());
    }

    // --- New-Feature-Keys-006: a key does NOT make its property functional (honest incompleteness). -
    #[test]
    fn keys_006_single_member_key_does_not_fire() {
        // Peter is the ONLY member of the key class, with two DIFFERENT hasName values. The key
        // constrains only DISTINCT individuals, so it never fires (no partner). The genuine
        // inconsistency needs owl:FunctionalProperty reasoning, which is OUTSIDE the EL fragment
        // (a declaration the extractor does not act on) — so the EL realiser is SOUNDLY consistent
        // here (honest incompleteness), never a fabricated merge/clash.
        let ttl = format!(
            "{PRE}
             :GriffinFamilyMember owl:hasKey ( :hasName ) .
             :Peter a :GriffinFamilyMember ; :hasName \"Peter\" , \"Kichwa-Tembo\" .
             :hasName a owl:FunctionalProperty ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent(), "a single-member key never fires (functional-prop clash is out of fragment)");
        assert!(r.same_as().is_empty(), "no spurious merge from a one-member key");
    }

    // --- A PARTIAL key match (only SOME key properties agree) must NOT merge (the negative test). -
    #[test]
    fn partial_key_match_does_not_merge() {
        // HasKey(C () (:p :q)); a and b share :p :v but DIFFER on :q — the multi-property key must
        // not fire on the partial agreement (the classic HasKey over-derivation trap).
        let ttl = format!(
            "{PRE}
             :C owl:hasKey ( :p :q ) .
             :a a :C ; :p :v ; :q :w .
             :b a :C ; :p :v ; :q :w2 ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (a, b) = (iri(&dict, "http://ex/a"), iri(&dict, "http://ex/b"));
        assert!(
            !r.same_as().contains(&(a, b)) && !r.same_as().contains(&(b, a)),
            "a partial key match must NOT merge — every key property must share a value"
        );
    }

    // --- The COMPLEMENT: an OBJECT key with EVERY property agreeing DOES merge. -----------------
    #[test]
    fn object_key_full_match_merges() {
        let ttl = format!(
            "{PRE}
             :C owl:hasKey ( :p :q ) .
             :a a :C ; :p :v ; :q :w .
             :b a :C ; :p :v ; :q :w ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (a, b) = (iri(&dict, "http://ex/a"), iri(&dict, "http://ex/b"));
        assert!(r.same_as().contains(&(a, b)), "both key properties agree ⇒ a owl:sameAs b");
        assert!(r.same_as().contains(&(b, a)));
    }

    // --- New-Feature-NegativeObjectPropertyAssertion-001: NPA + asserted positive ⇒ inconsistent. -
    #[test]
    fn npa_object_001_asserted_positive_is_inconsistent() {
        let ttl = format!(
            "{PRE}
             [ a owl:NegativePropertyAssertion ;
               owl:sourceIndividual :Peter ;
               owl:assertionProperty :hasSon ;
               owl:targetIndividual :Meg ] .
             :Peter :hasSon :Meg ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.is_inconsistent(), "¬(Peter hasSon Meg) contradicts the asserted positive");
    }

    // --- The NPA positive can be DERIVED (not asserted) — the object hasValue link. -------------
    #[test]
    fn npa_object_derived_positive_is_inconsistent() {
        // {a} ⊑ ∃p.{b} (owl:hasValue) DERIVES the link (a,b) ∈ R(p), which the object NPA clashes
        // with even though `a p b` is never an asserted triple.
        let ttl = format!(
            "{PRE}
             :a a [ owl:onProperty :p ; owl:hasValue :b ] .
             [ a owl:NegativePropertyAssertion ;
               owl:sourceIndividual :a ;
               owl:assertionProperty :p ;
               owl:targetIndividual :b ] ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.is_inconsistent(), "the DERIVED p-link to {{b}} clashes with ¬(a p b)");
    }

    // --- An NPA WITHOUT a matching positive is consistent (a clash needs the positive). ---------
    #[test]
    fn npa_object_no_positive_is_consistent() {
        let ttl = format!(
            "{PRE}
             [ a owl:NegativePropertyAssertion ;
               owl:sourceIndividual :Peter ;
               owl:assertionProperty :hasSon ;
               owl:targetIndividual :Meg ] ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent(), "an NPA alone (no positive) is satisfiable");
    }

    // --- New-Feature-NegativeDataPropertyAssertion-001: data NPA + asserted positive ⇒ inconsistent. -
    #[test]
    fn npa_data_001_asserted_positive_is_inconsistent() {
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             [ a owl:NegativePropertyAssertion ;
               owl:sourceIndividual :Meg ;
               owl:assertionProperty :hasAge ;
               owl:targetValue \"5\"^^xsd:integer ] .
             :Meg :hasAge \"5\"^^xsd:integer ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.is_inconsistent(), "¬(Meg hasAge 5) contradicts the asserted data value");
    }

    // --- A data NPA WITHOUT a matching positive value is consistent. ----------------------------
    #[test]
    fn npa_data_no_positive_is_consistent() {
        let ttl = format!(
            "{PRE}
             @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
             [ a owl:NegativePropertyAssertion ;
               owl:sourceIndividual :Meg ;
               owl:assertionProperty :hasAge ;
               owl:targetValue \"5\"^^xsd:integer ] .
             :Meg :hasAge \"6\"^^xsd:integer ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent(), "the asserted value differs from the negated one — consistent");
    }

    // --- WebOnt-differentFrom-001: owl:differentFrom is symmetric (asserted (a,b) ⊨ (b,a)). ------
    #[test]
    fn different_from_001_symmetric_readoff() {
        let ttl = format!("{PRE} :a owl:differentFrom :b .");
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        let (a, b) = (iri(&dict, "http://ex/a"), iri(&dict, "http://ex/b"));
        let df = r.different_from();
        assert!(df.contains(&(a, b)) && df.contains(&(b, a)), "differentFrom is symmetric");
    }

    // --- WebOnt-disjointWith-001: members of disjoint classes are owl:differentFrom. ------------
    #[test]
    fn disjoint_with_001_derived_different_from() {
        // a : A, b : B, A disjointWith B ⊨ a owl:differentFrom b (a derived nominal clash).
        let ttl = format!(
            "{PRE}
             :A owl:disjointWith :B .
             :a a :A , owl:Thing .
             :b a :B , owl:Thing ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent(), "a≠b: the individuals are distinct, so no single-individual clash");
        let (a, b) = (iri(&dict, "http://ex/a"), iri(&dict, "http://ex/b"));
        let df = r.different_from();
        assert!(df.contains(&(a, b)) && df.contains(&(b, a)), "disjoint members are differentFrom");
        assert!(!r.same_as().iter().any(|&(x, y)| (x, y) == (a, b)), "and never merged");
    }

    // --- A key merge that CONTRADICTS a derived disjointness clash ⇒ inconsistent. --------------
    #[test]
    fn key_merge_versus_derived_clash_is_inconsistent() {
        // The key forces a=b, but a∈A, b∈B, A⊓B⊑⊥ forces a≠b — a proof of both, so inconsistent.
        let ttl = format!(
            "{PRE}
             :A owl:disjointWith :B .
             owl:Thing owl:hasKey ( :ssn ) .
             :a a :A ; :ssn \"x\" .
             :b a :B ; :ssn \"x\" ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(r.is_inconsistent(), "key-derived sameAs vs derived differentFrom ⇒ inconsistent");
        assert!(r.same_as().is_empty() && r.different_from().is_empty());
    }

    // --- realize_graph materializes owl:differentFrom and stays idempotent. ---------------------
    #[test]
    fn realize_graph_materializes_different_from() {
        let ttl = format!(
            "{PRE}
             :A owl:disjointWith :B .
             :a a :A .
             :b a :B ."
        );
        let (mut dict, mut triples) = parse(&ttl);
        let rep = realize_graph(&mut dict, &mut triples);
        assert!(!rep.inconsistent);
        assert!(rep.emitted_different_from >= 1, "a owl:differentFrom b materialized");
        let (a, b) = (iri(&dict, "http://ex/a"), iri(&dict, "http://ex/b"));
        let df = iri(&dict, OWL_DIFFERENT_FROM);
        assert!(triples.contains(&[a, df, b]), "a owl:differentFrom b in the graph");
        let rep2 = realize_graph(&mut dict, &mut triples);
        assert_eq!(rep2.emitted_different_from, 0, "second call is idempotent");
    }

    // --- A malformed owl:hasKey (empty key list) is a fail-closed counted skip. -----------------
    #[test]
    fn malformed_empty_key_is_counted_skip() {
        let ttl = format!(
            "{PRE}
             :C owl:hasKey ( ) .
             :a a :C ; :p :v .
             :b a :C ; :p :v ."
        );
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        assert!(r.report().skipped_assertions >= 1, "an empty key list is a counted skip");
        let (a, b) = (iri(&dict, "http://ex/a"), iri(&dict, "http://ex/b"));
        assert!(
            !r.same_as().contains(&(a, b)),
            "a skipped key fabricates no merge"
        );
    }

    // --- An empty / TBox-only graph realises to nothing and is consistent. ----------------------
    #[test]
    fn tbox_only_graph_is_consistent_with_empty_realisation() {
        let ttl = format!("{PRE} :A rdfs:subClassOf :B .");
        let (dict, triples) = parse(&ttl);
        let r = realize(&dict, &triples);
        assert!(!r.is_inconsistent());
        assert!(r.type_assertions().is_empty());
        assert!(r.same_as().is_empty());
    }
}
