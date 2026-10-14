// sq-t5bne (epic sq-pbz04): map between spargebra CQ atoms and the rewriter's
// internal [`Atom`]s, and fold the rewritten UCQ back into a spargebra `Query`.
// sq-pbz04.3.1: B2 literal-object atoms — `Term::Lit` variant; the emitter
// maps `Term::Lit` back to `TermPattern::Literal`. Filter (B3) and VALUES (B4)
// pass-through: the rewritten UCQ body is wrapped in the original FILTER / VALUES so SPARQL
// evaluation semantics apply post-rewrite.
// sq-pbz04.3.6: body blank-node → fresh existential variable lifting. A blank
// node in a SPARQL query body (e.g. `_:b rdf:type :Person`) is a non-distinguished existential:
// it is scoped to the query, never an answer variable, and equivalent to a fresh non-shared
// non-distinguished variable. The correct mapping is `Term::Unbound(id)` — exactly the symbol
// PerfectRef's applicability condition governs. Each distinct blank-node LABEL within a CQ
// maps to one fresh id (a shared label → same id → `is_bound_var` sees sharing); different
// blank-node labels get different ids so they remain independent existentials. The freshness
// invariant is preserved by `seed_fresh` in perfectref.rs: it seeds the global counter above
// any `Unbound` id present in the input atoms, so blank-node ids never collide with the ids
// PerfectRef introduces during saturation. FAIL-CLOSED: blank nodes in a DISTINGUISHED
// (answer/projected) position are impossible in standard SPARQL (SELECT only projects named
// variables); the class-atom object guard (`rdf:type _:b` is not a named class) provides
// defence in depth. marker per shared contract.
//
// The seam is the same one sparq-vectors proves: produce a rewritten `spargebra::Query` so the
// engine — planner, executor — runs the UCQ unchanged (no store/planner changes). A UCQ folds
// to a left-deep tree of `Union`s of `Bgp`s, wrapped in the ORIGINAL projection (+ Distinct),
// then re-wrapped in any collected FILTER/VALUES from the broadened shapes.
//
// This whole module is behind `experimental` (it is only reachable from the gated rewriter).

#![cfg(feature = "experimental")]

use crate::cq::{ConjunctiveQuery, CqError, ValuesBlock};
use crate::dllite::rdf_type_iri;
use crate::perfectref::{Atom, Cq, Term};
use crate::dllite::Role;
use rustc_hash::FxHashMap;
use spargebra::algebra::GraphPattern;
use spargebra::term::{GroundTerm, Literal, NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable};

/// Map the gated CQ's triple patterns to internal atoms + the answer-variable names. Returns the
/// (atoms, answer) pair PerfectRef consumes. Errors (OutOfScope) if a triple pattern is not a
/// DL-Lite atom in a way the CQ-shape gate did not already reject (defence in depth: e.g. a
/// quoted triple or a blank node in a class-name position).
///
/// B2: literals in role-atom object positions are now accepted (mapped to `Term::Lit`).
/// Soundness: a literal constant is never an unbound position, so no existential inclusion is
/// applicable — the applicability condition is untouched. Role inclusions carry the literal
/// constant unchanged. sq-pbz04.3.1
///
/// Body blank-node → fresh existential variable (sq-pbz04.3.6): a blank node in a SPARQL
/// query body (`_:b rdf:type :Person`) is a non-distinguished existential. Each distinct
/// blank-node label maps to one `Term::Unbound(id)`, with the SAME label → SAME id within this
/// CQ (so sharing is detected correctly by `is_bound_var`). The freshness invariant is: ids
/// assigned here start at 0 and `seed_fresh` in perfectref.rs scans the input atoms and seeds
/// its `Fresh` counter strictly above the max id present — so no collision between blank-node
/// ids and the ids PerfectRef introduces during saturation. sq-pbz04.3.6
pub fn cq_to_atoms(cq: &ConjunctiveQuery) -> Result<(Vec<Atom>, Vec<String>), CqError> {
    let rdf_type = rdf_type_iri();
    let mut atoms = Vec::with_capacity(cq.atoms.len());
    // Blank-node label → Unbound id map. Shared labels → same id → `is_bound_var` detects
    // sharing and correctly treats the shared position as bound (so the existential-introducing
    // inclusions' applicability condition is evaluated correctly). sq-pbz04.3.6
    let mut bnode_ids: FxHashMap<String, u32> = FxHashMap::default();
    let mut next_bnode_id: u32 = 0;
    for tp in &cq.atoms {
        let pred = match &tp.predicate {
            NamedNodePattern::NamedNode(n) => n.clone(),
            // The gate already rejects variable predicates; keep the arm total/honest.
            NamedNodePattern::Variable(_) => {
                return Err(CqError::OutOfScope(
                    "variable predicate is not a DL-Lite atom".into(),
                ))
            }
        };
        if pred == rdf_type {
            // Class atom: subject is the arg, object must be a NAMED class.
            let arg = term_pattern_to_term_with_bnode(
                &tp.subject,
                &mut bnode_ids,
                &mut next_bnode_id,
            )?;
            let class = match &tp.object {
                TermPattern::NamedNode(c) => c.as_str().to_string(),
                // Defence in depth: `rdf:type _:b` is rejected — a blank node is not a named
                // class. This cannot be a distinguished existential in the class position.
                // sq-pbz04.3.6
                TermPattern::BlankNode(_) => {
                    return Err(CqError::OutOfScope(
                        "rdf:type object is a blank node, not a named class \
                         (fail-closed sq-pbz04.3.6: blank-node class names are not DL-Lite)"
                            .into(),
                    ))
                }
                _ => {
                    return Err(CqError::OutOfScope(
                        "rdf:type object must be a named class for DL-Lite rewriting".into(),
                    ))
                }
            };
            atoms.push(Atom::Class { class, arg });
        } else {
            // Role atom: subject + object are terms, predicate is the role IRI.
            // B2: literals are now permitted in object position. sq-pbz04.3.1
            // sq-pbz04.3.6: blank nodes in subject/object positions are now existentials.
            let s = term_pattern_to_term_with_bnode(
                &tp.subject,
                &mut bnode_ids,
                &mut next_bnode_id,
            )?;
            let o = term_pattern_to_term_with_bnode(
                &tp.object,
                &mut bnode_ids,
                &mut next_bnode_id,
            )?;
            atoms.push(Atom::Role {
                role: Role::named(pred.as_str()),
                s,
                o,
            });
        }
    }
    let answer = cq.distinguished.iter().map(|v| v.as_str().to_string()).collect();
    Ok((atoms, answer))
}

/// Map a spargebra `TermPattern` to an internal `Term`, resolving blank nodes to fresh
/// existential `Unbound` ids via `bnode_ids` (the label-to-id map for this CQ).
///
/// - Variables → `Var` (by name).
/// - Named nodes → `Const` (IRI string).
/// - Literals → `Lit` (lexical, datatype, lang) — B2 sq-pbz04.3.1.
/// - Blank nodes → `Unbound(id)` — B for body blank-node lifting sq-pbz04.3.6.
///   A blank node label seen for the first time in this CQ gets a new id; repeated occurrences
///   of the same label map to the SAME id (shared existential, counts as bound in
///   `is_bound_var`). Soundness: a shared blank node is a join condition that the rewriter must
///   preserve; `is_bound_var` then correctly BLOCKS the existential applicability condition on
///   that position (exactly as it does for a shared named variable).
/// - Quoted triples → `OutOfScope` (RDF-star, out of DL-Lite scope).
fn term_pattern_to_term_with_bnode(
    t: &TermPattern,
    bnode_ids: &mut FxHashMap<String, u32>,
    next_id: &mut u32,
) -> Result<Term, CqError> {
    match t {
        TermPattern::Variable(v) => Ok(Term::Var(v.as_str().to_string())),
        TermPattern::NamedNode(n) => Ok(Term::Const(n.as_str().to_string())),
        // sq-pbz04.3.6: body blank nodes → fresh existential `Unbound`. Shared labels →
        // same id → `is_bound_var` detects the shared position as bound.
        // sq-pbz04.3.6
        TermPattern::BlankNode(b) => {
            let label = b.as_str().to_string();
            let id = *bnode_ids.entry(label).or_insert_with(|| {
                let id = *next_id;
                *next_id += 1;
                id
            });
            Ok(Term::Unbound(id))
        }
        // B2: literals in role-atom object positions are now accepted. Store as three separate
        // strings (lexical value, datatype IRI, optional language tag) so `Term` can derive
        // `Ord`. The emitter reconstructs the original `oxrdf::Literal` from these fields.
        // sq-pbz04.3.1
        TermPattern::Literal(lit) => {
            let lexical = lit.value().to_string();
            let datatype = lit.datatype().as_str().to_string();
            let lang = lit.language().map(|l| l.to_string());
            Ok(Term::Lit(lexical, datatype, lang))
        }
        // `TermPattern::Triple` exists because spargebra is built with `sparql-12` (workspace
        // dep). Match it directly — a `cfg(feature = "sparql-12")` would test THIS crate.
        TermPattern::Triple(_) => Err(CqError::OutOfScope(
            "RDF-star quoted-triple term is out of DL-Lite scope".into(),
        )),
    }
}

/// Fold a UCQ (the rewritten CQ set) into one spargebra `GraphPattern` body: a tree of `Union`s
/// of `Bgp`s, in a DETERMINISTIC order (CQs sorted by their textual atom signature) so the
/// rewrite output is stable run-to-run. An empty UCQ is impossible (the input CQ is always
/// present), but is handled defensively as an empty BGP.
///
/// B3/B4 pass-through: `filter_exprs` and `values_blocks` from the original `ConjunctiveQuery`
/// are re-applied around the rewritten UCQ body. FILTER wraps the body (innermost); VALUES
/// folds in as inner joins with the UCQ. sq-pbz04.3.1
pub fn ucq_to_pattern(mut ucq: Vec<Cq>, cq: &ConjunctiveQuery) -> GraphPattern {
    // Deterministic order: sort by a stable signature of each CQ's normalised atoms.
    ucq.sort_by_cached_key(cq_signature);
    let mut bgps: Vec<GraphPattern> = ucq.iter().map(cq_to_bgp).collect();
    bgps.dedup(); // structurally-identical disjuncts collapse (sound: UCQ is a set).
    let base = match bgps.len() {
        0 => GraphPattern::Bgp { patterns: vec![] },
        1 => bgps.pop().unwrap(),
        _ => {
            // Left-deep Union fold.
            let mut iter = bgps.into_iter();
            let mut acc = iter.next().unwrap();
            for next in iter {
                acc = GraphPattern::Union {
                    left: Box::new(acc),
                    right: Box::new(next),
                };
            }
            acc
        }
    };

    // B4: re-apply VALUES blocks as inner joins.
    let with_values = apply_values_blocks(base, &cq.values_blocks);

    // B3: re-apply FILTER expressions wrapping the body.
    apply_filter_exprs(with_values, &cq.filter_exprs)
}

/// BRANCH-AWARE fold for a multi-branch UCQ where the branches carry PER-BRANCH modifiers
/// (FILTER/VALUES) — the general case sq-sg542 closes. Each `(disjuncts, cq)` pair is folded by
/// [`ucq_to_pattern`] into that branch's OWN sub-body (the union of its PerfectRef disjuncts,
/// wrapped in ONLY that branch's `filter_exprs` / `values_blocks`), and the branch sub-bodies are
/// then combined by a top-level left-deep `Union` in SOURCE-BRANCH ORDER (deterministic).
///
/// SOUNDNESS — per-branch modifier isolation. Because each branch's FILTER/VALUES wraps only that
/// branch's sub-union, a FILTER owned by branch *i* constrains branch *i* alone and can NEVER leak
/// onto branch *j ≠ i* (nor is a branch's modifier ever silently dropped — the old single-passthrough
/// emitter bug this bead fixes). FILTER and the VALUES-join both distribute over union, so wrapping
/// the sub-union is equivalent to wrapping each disjunct; and certain answers distribute over union
/// in DL-Lite_R (B1), so the union of the per-branch answer sets is exactly the UCQ's certain-answer
/// set. The distinguished-only discipline (B3/B4), checked per branch by the gate, guarantees every
/// modifier reads only projected variables bound in every disjunct of its branch. sq-sg542
pub fn ucq_to_pattern_per_branch(branches: Vec<(Vec<Cq>, &ConjunctiveQuery)>) -> GraphPattern {
    let mut branch_bodies: Vec<GraphPattern> = branches
        .into_iter()
        .map(|(disjuncts, cq)| ucq_to_pattern(disjuncts, cq))
        .collect();
    match branch_bodies.len() {
        0 => GraphPattern::Bgp { patterns: vec![] },
        1 => branch_bodies.pop().unwrap(),
        _ => {
            // Left-deep Union fold over the branch sub-bodies (source-branch order preserved).
            let mut iter = branch_bodies.into_iter();
            let mut acc = iter.next().unwrap();
            for next in iter {
                acc = GraphPattern::Union {
                    left: Box::new(acc),
                    right: Box::new(next),
                };
            }
            acc
        }
    }
}

/// Wrap `body` in `Join { body, Values { variables, bindings } }` for each VALUES block.
/// SPARQL VALUES as a join pattern is the standard way to inject constant bindings.
fn apply_values_blocks(body: GraphPattern, blocks: &[ValuesBlock]) -> GraphPattern {
    blocks.iter().fold(body, |acc, vb| {
        // Convert `Vec<Vec<GroundTerm>>` back to `Vec<Vec<Option<GroundTerm>>>` as spargebra expects.
        let bindings: Vec<Vec<Option<GroundTerm>>> = vb.bindings.iter()
            .map(|row| row.iter().map(|gt| Some(gt.clone())).collect())
            .collect();
        GraphPattern::Join {
            left: Box::new(acc),
            right: Box::new(GraphPattern::Values {
                variables: vb.variables.clone(),
                bindings,
            }),
        }
    })
}

/// Wrap `body` in `Filter { expr, inner: body }` for each collected FILTER expression.
/// Multiple FILTERs are applied innermost-first (the order they were collected).
fn apply_filter_exprs(body: GraphPattern, exprs: &[spargebra::algebra::Expression]) -> GraphPattern {
    exprs.iter().fold(body, |acc, expr| GraphPattern::Filter {
        expr: expr.clone(),
        inner: Box::new(acc),
    })
}

/// A stable textual signature of a CQ for deterministic UCQ ordering.
fn cq_signature(cq: &Cq) -> String {
    let mut parts: Vec<String> = cq.atoms.iter().map(atom_signature).collect();
    parts.sort();
    parts.join(" ")
}

fn atom_signature(a: &Atom) -> String {
    match a {
        Atom::Class { class, arg } => format!("C|{}|{}", class, term_sig(arg)),
        Atom::Role { role, s, o } => format!(
            "R|{}|{}|{}|{}",
            role.iri,
            role.inverse,
            term_sig(s),
            term_sig(o)
        ),
    }
}

fn term_sig(t: &Term) -> String {
    match t {
        Term::Var(v) => format!("?{}", v),
        Term::Const(c) => format!("<{}>", c),
        Term::Unbound(_) => "_".to_string(),
        // B2: literals get a distinct stable signature.
        Term::Lit(lv, _ld, Some(ll)) => format!("\"{}\"@{}", lv, ll),
        Term::Lit(lv, ld, None) => format!("\"{}\"^^<{}>", lv, ld),
    }
}

/// Build a `Bgp` from one CQ, mapping each atom to a triple pattern. Unbound `_` terms become
/// deterministically-named non-distinguished variables (`__ql<n>`) — the standard SPARQL
/// encoding of an existential BGP variable. The naming is per-Bgp so two disjuncts never clash.
///
/// SHARING (load-bearing, sq-pbz04.3.6): the SAME internal `Unbound(id)` MUST map to
/// the SAME emitted variable across all its occurrences in the CQ — a repeated id is a shared
/// existential (a JOIN through an intermediate node), e.g. the shared body blank node `_:a` in
/// `?X :p _:a . _:a :r ?Y`, or a fresh existential PerfectRef reuses across atoms. Assigning a
/// fresh name PER OCCURRENCE would drop the join and emit a CARTESIAN product — an unsound
/// OVER-approximation of the certain answers. So the id→name map is memoised per Bgp: distinct
/// ids get distinct compact names, equal ids share one name.
fn cq_to_bgp(cq: &Cq) -> GraphPattern {
    let rdf_type = NamedNodePattern::NamedNode(rdf_type_iri());
    let mut fresh_ids: FxHashMap<u32, u32> = FxHashMap::default();
    let mut next_fresh = 0u32;
    let mut fresh_name = |id: u32| {
        // Memoise: an internal Unbound id maps to ONE stable per-Bgp variable name (small local
        // index, so emitted names are compact and order-stable). Equal ids → the same variable,
        // preserving the shared-existential JOIN; distinct ids → distinct variables.
        let n = *fresh_ids.entry(id).or_insert_with(|| {
            let cur = next_fresh;
            next_fresh += 1;
            cur
        });
        Variable::new_unchecked(format!("__ql{}", n))
    };
    let mut patterns = Vec::with_capacity(cq.atoms.len());
    for atom in &cq.atoms {
        match atom {
            Atom::Class { class, arg } => {
                patterns.push(TriplePattern {
                    subject: term_to_pattern(arg, &mut fresh_name),
                    predicate: rdf_type.clone(),
                    object: TermPattern::NamedNode(NamedNode::new_unchecked(class.clone())),
                });
            }
            Atom::Role { role, s, o } => {
                // Roles are stored forward (the rewriter forward-normalises inverses), so emit
                // the named property directly between subject and object.
                patterns.push(TriplePattern {
                    subject: term_to_pattern(s, &mut fresh_name),
                    predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(
                        role.iri.clone(),
                    )),
                    object: term_to_pattern(o, &mut fresh_name),
                });
            }
        }
    }
    GraphPattern::Bgp { patterns }
}

fn term_to_pattern(t: &Term, fresh: &mut impl FnMut(u32) -> Variable) -> TermPattern {
    match t {
        Term::Var(v) => TermPattern::Variable(Variable::new_unchecked(v.clone())),
        Term::Const(c) => TermPattern::NamedNode(NamedNode::new_unchecked(c.clone())),
        Term::Unbound(id) => TermPattern::Variable(fresh(*id)),
        // B2: emit literals back. Reconstruct from stored (lexical, datatype, lang) triple.
        // Language-tagged literals: new_language_tagged_literal(value, lang).
        // Plain/typed literals: new_typed_literal(value, NamedNode(datatype)).
        // sq-pbz04.3.1
        Term::Lit(lv, ld, lang_opt) => {
            let lit = if let Some(lang) = lang_opt {
                Literal::new_language_tagged_literal(lv.clone(), lang.clone())
                    .unwrap_or_else(|_| Literal::new_simple_literal(lv.clone()))
            } else {
                Literal::new_typed_literal(
                    lv.clone(),
                    NamedNode::new_unchecked(ld.clone()),
                )
            };
            TermPattern::Literal(lit)
        }
    }
}
