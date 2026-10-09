use super::*;

// ---- Spatial FILTER pushdown (sq-mg9) ------------------------------------------
//
// Recognise a `geof:` spatial FILTER over an indexed geometry variable and, when a
// SpatialProvider is installed, pre-restrict the bindings to the index's candidate
// SUPERSET before the exact `geof:` refinement runs in `apply_filter`. The engine
// stays geometry-free: it only lifts the geometry variable and the CONSTANT operands
// out of the algebra and forwards them to the provider; all geometry math is the
// provider's (sparq-geo's). Correctness is by construction — the candidate set is a
// superset, the residual `geof:` FILTER is NOT removed, so the result is identical to
// the post-hoc path.

/// The geof: IRIs the planner can push down. (The engine cannot depend on
/// sparq-geo's `vocab`, so the IRIs are mirrored here; they are W3C-fixed.)
pub(super) const GEOF_DISTANCE: &str = "http://www.opengis.net/def/function/geosparql/distance";
pub(super) const GEOF_SF_WITHIN: &str = "http://www.opengis.net/def/function/geosparql/sfWithin";
pub(super) const GEOF_SF_INTERSECTS: &str = "http://www.opengis.net/def/function/geosparql/sfIntersects";
#[cfg(feature = "spatial-exact-pushdown")]
pub(super) const GEOF_SF_CONTAINS: &str = "http://www.opengis.net/def/function/geosparql/sfContains";

/// A recognised pushable spatial FILTER: the geometry variable to restrict, plus an
/// OWNED description of the index query (so it outlives the borrowed algebra). The
/// `SpatialQuery` handed to the provider borrows from the owned strings here.
pub(super) struct SpatialPushdown {
    pub(super) geo_var: Variable,
    pub(super) kind: SpatialKind,
}

pub(super) enum SpatialKind {
    DistanceWithin { point_wkt: String, radius: f64, unit_iri: String, inclusive: bool },
    BboxIntersects {
        arg_wkt: String,
        /// (sq-lk3aw.4) `true` iff the recognised FILTER is the
        /// WITHIN-REGION orientation — `geof:sfWithin(?g, REGION)` or its Simple
        /// Features converse `geof:sfContains(REGION, ?g)` — the one predicate an
        /// installed provider may certify EXACTLY via
        /// [`SpatialProvider::candidates_exact`](crate::SpatialProvider::candidates_exact).
        /// `false` (sfIntersects) keeps the superset + residual-FILTER path.
        #[cfg(feature = "spatial-exact-pushdown")]
        within_region: bool,
    },
}

/// The `geo:wktLiteral` lexical form of a constant operand, if it is one. (The engine
/// does not validate the WKT — the provider parses it and declines on failure.)
pub(super) fn wkt_const(e: &Expression) -> Option<&str> {
    const WKT_LITERAL: &str = "http://www.opengis.net/ont/geosparql#wktLiteral";
    match e {
        Expression::Literal(l) if l.datatype().as_str() == WKT_LITERAL => Some(l.value()),
        _ => None,
    }
}

/// A `geof:distance(?g, point[, unit])` call → `(geo_var, point_wkt, unit_iri)`, with
/// `?g` the geometry variable and the OTHER geometry operand a constant wktLiteral. The
/// unit defaults to metre when omitted (GeoSPARQL `geof:distance` is arity-3, but be
/// lenient). Symmetric in the two geometry args.
pub(super) fn match_geof_distance(args: &[Expression]) -> Option<(Variable, String, String)> {
    const UOM_METRE: &str = "http://www.opengis.net/def/uom/OGC/1.0/metre";
    if args.len() != 2 && args.len() != 3 {
        return None;
    }
    let unit = if args.len() == 3 {
        match &args[2] {
            Expression::NamedNode(nn) => nn.as_str().to_string(),
            _ => return None,
        }
    } else {
        UOM_METRE.to_string()
    };
    let var_of = |e: &Expression| match e {
        Expression::Variable(v) => Some(v.clone()),
        _ => None,
    };
    // (?g, point) or (point, ?g): the geometry var on either side.
    for (a, b) in [(&args[0], &args[1]), (&args[1], &args[0])] {
        if let (Some(v), Some(pt)) = (var_of(a), wkt_const(b)) {
            return Some((v, pt.to_string(), unit));
        }
    }
    None
}

/// Recognises a pushable spatial FILTER. Two shapes:
///
/// * `geof:distance(?g, point, unit) OP radius` with `OP ∈ {<, <=}` (the bound on the
///   non-constant side) → a distance-within window;
/// * `geof:sfWithin(?g, box)` / `geof:sfIntersects(?g, box)` used directly as the FILTER
///   → a bbox/window scan.
///
/// `geof:sfContains/Overlaps/Touches/Crosses/Equals/Disjoint` and distance with `>`/`>=`
/// (an unbounded EXTERIOR, no finite window) stay post-hoc — see the report.
pub(super) fn recognise_spatial(e: &Expression) -> Option<SpatialPushdown> {
    use spargebra::algebra::Function;
    // distance(...) OP radius — match the comparison, then the distance call inside.
    let distance_cmp = |call: &Expression, radius: &Expression, inclusive: bool| -> Option<SpatialPushdown> {
        let Expression::FunctionCall(Function::Custom(nn), cargs) = call else { return None };
        if nn.as_str() != GEOF_DISTANCE {
            return None;
        }
        let r = match radius {
            Expression::Literal(l) => l.value().parse::<f64>().ok()?,
            _ => return None,
        };
        if !r.is_finite() || r < 0.0 {
            return None;
        }
        let (geo_var, point_wkt, unit_iri) = match_geof_distance(cargs)?;
        Some(SpatialPushdown { geo_var, kind: SpatialKind::DistanceWithin { point_wkt, radius: r, unit_iri, inclusive } })
    };
    match e {
        // A finite within-window needs distance on the SMALLER side: `distance(...) < r`
        // or the mirror `r > distance(...)`. The opposite orientation (`r < distance` /
        // `distance > r`) is an unbounded EXTERIOR — NOT pushable — so each arm matches
        // exactly ONE operand orientation (asymmetric on purpose; the earlier symmetric
        // `.or_else` wrongly recognised the exterior as a within-window).
        Expression::Less(l, r) => distance_cmp(l, r, false), // distance(...) < r
        Expression::LessOrEqual(l, r) => distance_cmp(l, r, true), // distance(...) <= r
        Expression::Greater(l, r) => distance_cmp(r, l, false), // r > distance(...)
        Expression::GreaterOrEqual(l, r) => distance_cmp(r, l, true), // r >= distance(...)
        // geof:sfWithin(?g, box) / geof:sfIntersects(?g, box) as the whole FILTER.
        Expression::FunctionCall(Function::Custom(nn), cargs) => {
            let iri = nn.as_str();
            if (iri == GEOF_SF_WITHIN || iri == GEOF_SF_INTERSECTS) && cargs.len() == 2 {
                if let (Expression::Variable(v), Some(arg)) = (&cargs[0], wkt_const(&cargs[1])) {
                    return Some(SpatialPushdown {
                        geo_var: v.clone(),
                        kind: SpatialKind::BboxIntersects {
                            arg_wkt: arg.to_string(),
                            #[cfg(feature = "spatial-exact-pushdown")]
                            within_region: iri == GEOF_SF_WITHIN,
                        },
                    });
                }
            }
            // (sq-lk3aw.4) `geof:sfContains(REGION, ?g)` — the constant-region-
            // FIRST operand order. Simple Features defines `contains(a, b) ⇔ within(b, a)`,
            // so this is the SAME within-region pushdown as `sfWithin(?g, REGION)` with the
            // operands swapped: `within(?g, REGION) ⟹ intersects(?g, REGION) ⟹ their AABBs
            // intersect`, hence the bbox candidate superset is valid for it too. Previously
            // this orientation was never recognised and never pushed down at all.
            #[cfg(feature = "spatial-exact-pushdown")]
            if iri == GEOF_SF_CONTAINS && cargs.len() == 2 {
                if let (Some(arg), Expression::Variable(v)) = (wkt_const(&cargs[0]), &cargs[1]) {
                    return Some(SpatialPushdown {
                        geo_var: v.clone(),
                        kind: SpatialKind::BboxIntersects {
                            arg_wkt: arg.to_string(),
                            within_region: true,
                        },
                    });
                }
            }
            None
        }
        _ => None,
    }
}

/// If `b` binds the recognised geometry variable, restrict its rows to the provider's
/// candidate superset over that variable, in place. Returns `true` when the pushdown
/// fired (an index was installed AND served candidates) so the caller knows the residual
/// FILTER still refines a SMALLER input set. A miss (no index, declined, or unbound var)
/// is a no-op returning `false` — the residual FILTER then scans every row, unchanged.
pub(super) fn apply_spatial_pushdown(graph: &Graph, b: &mut Bindings, pd: &SpatialPushdown) -> bool {
    use crate::SpatialQuery;
    let Some(col) = b.col(&pd.geo_var) else { return false };
    let Some(idx) = spatial::active() else { return false };
    let query = match &pd.kind {
        SpatialKind::DistanceWithin { point_wkt, radius, unit_iri, inclusive } => {
            SpatialQuery::DistanceWithin { point_wkt, radius: *radius, unit_iri, inclusive: *inclusive }
        }
        SpatialKind::BboxIntersects { arg_wkt, .. } => SpatialQuery::BboxIntersects { arg_wkt },
    };
    let Some(cands) = idx.candidates(&query) else { return false };
    // Map the candidate TERMS to dictionary ids for an O(1) membership test on the
    // scanned column. A candidate term absent from the dict can never bind here, so
    // dropping it from the id-set is safe (and keeps the set a superset of the matches).
    let cand_ids: FxHashSet<Id> = cands.iter().filter_map(|t| graph.id_of(t)).collect();
    // The keep predicate (both branches below are IDENTICAL): keep a row when its
    // binding is a candidate (an indexed match) OR the index has NO opinion on it (NOT
    // indexed — e.g. bound via a non-`geo:asWKT` predicate, a non-geographic CRS, or a
    // different graph). The residual `geof:` FILTER then judges every kept row, so a
    // geometry the index never saw is NEVER silently dropped. This makes the result
    // identical to the post-hoc path no matter which subset the index covers.
    //
    // FAST PATH: when the provider can give the ID-LEVEL indexed universe resolved
    // against THIS graph's dict (freshness matched), `is_indexed(id)` is a pure
    // `FxHashSet<Id>` lookup — ZERO per-row `Term` materialisation. `indexed.contains(&id)`
    // equals `is_indexed(&graph.dict.term(id))` by the provider's contract, so the verdict
    // is unchanged.
    let dict_ptr = std::ptr::from_ref(&graph.dict) as usize;
    if let Some(indexed) = idx.indexed_ids(dict_ptr) {
        b.rows.retain(|row| {
            let id = row[col];
            // candidate (indexed match) OR not in the indexed universe -> keep.
            cand_ids.contains(&id) || !indexed.contains(&id)
        });
        return true;
    }
    // SLOW FALLBACK (no fresh id-level universe): memoise the per-id not-indexed check to
    // avoid re-materialising a term per row. Produces the SAME keep verdict as the fast path.
    let mut verdict: FxHashMap<Id, bool> = FxHashMap::default();
    b.rows.retain(|row| {
        let id = row[col];
        if cand_ids.contains(&id) {
            return true; // an indexed candidate (kept; exact FILTER refines)
        }
        *verdict.entry(id).or_insert_with(|| {
            // Keep iff the index is NOT authoritative over this binding.
            match term_of_id(graph, id) {
                Some(t) => !idx.is_indexed(&t),
                None => true, // no term (synthetic / unbound) — index can't rule it out
            }
        })
    });
    true
}

/// (sq-lk3aw.4) Outcome of the EXACT-candidate spatial pushdown attempt.
#[cfg(feature = "spatial-exact-pushdown")]
pub(super) enum ExactPushdown {
    /// No exact certification happened (not the within-region shape, no provider,
    /// unbound variable, or the provider declined): NOTHING was touched — the caller
    /// runs the superset pushdown + residual FILTER exactly as before.
    Declined,
    /// Rows were restricted to `certified-exact ∪ not-indexed`, but at least one
    /// surviving row's binding is NOT certified (outside the indexed universe): the
    /// residual FILTER MUST still run — it is an identity on the certified rows and
    /// the SOLE judge of the not-indexed ones.
    Partial,
    /// Every surviving row's binding is in the provider's certified-exact set: the
    /// residual FILTER is provably an identity and may be skipped.
    AllCertified,
}

/// (sq-lk3aw.4) EXACT-candidate spatial pushdown: for a recognised
/// WITHIN-REGION FILTER (`geof:sfWithin(?g, REGION)` / `geof:sfContains(REGION, ?g)`),
/// restrict `b`'s rows to the provider's CERTIFIED-EXACT answer set (keeping every
/// not-indexed binding) and report whether the residual `geof:` FILTER may be skipped.
///
/// EXACTNESS / SOUNDNESS ARGUMENT — why skipping the residual FILTER is safe, and
/// ONLY on [`ExactPushdown::AllCertified`]:
///
/// * [`SpatialProvider::candidates_exact`](crate::SpatialProvider::candidates_exact)
///   returning `Some(v)` CERTIFIES that `v` is EXACTLY the set of INDEXED geometry
///   bindings for which the recognised predicate evaluates to `true` — no false
///   positives and no false negatives, but ONLY over the indexed universe
///   ([`SpatialProvider::is_indexed`](crate::SpatialProvider::is_indexed)). The
///   certificate says NOTHING about a binding the index never saw.
/// * The retain below keeps a row iff its binding is (a) in the certified set — an
///   indexed TRUE row the residual FILTER would also keep — or (b) NOT indexed — a
///   row the certificate does not cover, kept for the residual FILTER to judge,
///   exactly as the superset path keeps it. An indexed row NOT in the certified set
///   is an indexed FALSE row the residual FILTER would drop: dropping it here is the
///   same verdict, taken earlier.
/// * If every surviving row entered via (a), the residual FILTER would keep each of
///   them (all certified TRUE), so running it cannot change the multiset — it is an
///   IDENTITY and skipping it is result-identical (`AllCertified`). This includes the
///   empty relation.
/// * If ANY surviving row entered via (b), skipping the residual FILTER could admit a
///   false positive (a not-indexed binding that does NOT satisfy the predicate) — a
///   SOUNDNESS bug. So the residual runs (`Partial`): an identity on the (a)-rows and
///   the judge of the (b)-rows, keeping the result byte-identical to the post-hoc
///   plan while the exact restriction still shrank the residual's input.
///
/// The certificate is trusted the same way `candidates`' superset contract is: the
/// provider asserts its exact refinement agrees with the registered `geof:` function
/// semantics (sparq-geo pins that equivalence in its `topology_index` tests).
#[cfg(feature = "spatial-exact-pushdown")]
pub(super) fn apply_spatial_pushdown_exact(graph: &Graph, b: &mut Bindings, pd: &SpatialPushdown) -> ExactPushdown {
    let SpatialKind::BboxIntersects { arg_wkt, within_region: true } = &pd.kind else {
        return ExactPushdown::Declined;
    };
    let Some(col) = b.col(&pd.geo_var) else { return ExactPushdown::Declined };
    let Some(idx) = spatial::active() else { return ExactPushdown::Declined };
    let query = crate::SpatialExactQuery::WithinRegion { region_wkt: arg_wkt };
    let Some(exact) = idx.candidates_exact(&query) else { return ExactPushdown::Declined };
    // A certified term absent from the graph dict can never bind here, so dropping it
    // keeps `exact_ids` exactly the certified answers that can appear in `b` (mirrors
    // the superset path's `cand_ids` mapping).
    let exact_ids: FxHashSet<Id> = exact.iter().filter_map(|t| graph.id_of(t)).collect();
    let mut all_certified = true;
    // Same two retain branches as `apply_spatial_pushdown` (id-level fast path when the
    // provider vouches its id universe is fresh for THIS dict, per-row fallback
    // otherwise), same keep verdict in both: certified (a) or not-indexed (b).
    let dict_ptr = std::ptr::from_ref(&graph.dict) as usize;
    if let Some(indexed) = idx.indexed_ids(dict_ptr) {
        b.rows.retain(|row| {
            let id = row[col];
            if exact_ids.contains(&id) {
                return true; // (a) certified TRUE — the residual FILTER would keep it too
            }
            let keep = !indexed.contains(&id); // (b) keep not-indexed for the residual FILTER
            all_certified &= !keep;
            keep
        });
    } else {
        let mut verdict: FxHashMap<Id, bool> = FxHashMap::default();
        b.rows.retain(|row| {
            let id = row[col];
            if exact_ids.contains(&id) {
                return true;
            }
            let keep = *verdict.entry(id).or_insert_with(|| match term_of_id(graph, id) {
                Some(t) => !idx.is_indexed(&t),
                None => true, // no term (synthetic / unbound) — the index can't rule it out
            });
            all_certified &= !keep;
            keep
        });
    }
    if all_certified { ExactPushdown::AllCertified } else { ExactPushdown::Partial }
}

/// The graph-dictionary term for `id`, if any. (A standalone helper so the spatial
/// pushdown can resolve a binding without a `LocalVocab`; bindings reaching a pushable
/// scan-bound geometry FILTER are graph-dict ids.)
pub(super) fn term_of_id(graph: &Graph, id: Id) -> Option<Term> {
    (id != NO_ID && !dict::is_inline(id)).then(|| graph.dict.term(id))
}
