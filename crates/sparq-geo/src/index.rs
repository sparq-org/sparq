//! [`GeoIndex`]: an R-tree over the geometries of a sparq [`Graph`].
//!
//! ## Extraction (GeoSPARQL core RDF shape)
//!
//! [`GeoIndex::build`] scans the graph's `geo:asWKT` triples — in the DEFAULT
//! graph and in every NAMED graph ([`Graph::named`]). For each
//! `(?g, geo:asWKT, "..."^^geo:wktLiteral)` the indexed ENTITY is every
//! feature `?f` with `?f geo:hasGeometry ?g` or `?f geo:hasDefaultGeometry
//! ?g` (resolved within the same graph); when a geometry node has no owning
//! feature the geometry node itself is the entity (datasets often attach
//! `geo:asWKT` straight to the thing). Each [`Entry`] records which graph it
//! came from ([`Entry::graph`], `None` for the default graph) and its
//! `geo:asWKT` subject ([`Entry::node`]). Literals that fail to parse, are
//! EMPTY, carry a non-geographic CRS, or are not `geo:wktLiteral`-typed are
//! counted in [`GeoIndex::skipped`] rather than aborting the build.
//!
//! ## Index design
//!
//! Entries live in a slotted `Vec` (slots are tombstoned by incremental
//! deletes and reused by inserts); the R-tree (`rstar::RTree`, bulk-loaded —
//! packed STR build, O(n log n)) stores one `{entry slot, AABB}` leaf per
//! geometry, in LONG/LAT DEGREE space. Queries prune by bounding box in the
//! tree and then refine against the actual geometry:
//!
//! - [`within_distance`](GeoIndex::within_distance) — converts the metre
//!   radius into one or two pole-safe long/lat windows (a ball CROSSING THE
//!   ANTIMERIDIAN is split into two windows, one per side of ±180°), walks
//!   `locate_in_envelope_intersecting` per window, refines with the true
//!   great-circle distance ([`geof::point_to_geometry_meters`]).
//! - [`nearest`](GeoIndex::nearest) — expanding-radius search over
//!   `within_distance` (radius ×4 per round, seeded from the tree's own
//!   extent), exact under the same great-circle metric (and antimeridian-safe
//!   through `within_distance`).
//! - [`intersects`](GeoIndex::intersects) — window query on the argument's
//!   AABB refined with `geo::Intersects` (degree space). The argument's OWN
//!   coordinates are taken as written: a geometry whose WKT runs past ±180°
//!   is not wrapped (the refinement is planar in degree space either way).
//!
//! ## Incremental maintenance
//!
//! [`apply_delta`](GeoIndex::apply_delta) mirrors a [`Graph::apply_delta`]
//! batch into the index without a rebuild: the geometry nodes touched by the
//! batch are re-extracted from the (post-delta) graph, their old entries
//! removed from the R-tree (`rstar` incremental remove) and the new state
//! inserted (`rstar` incremental insert) — O(batch × log n) instead of the
//! O(n log n) rebuild. Deltas apply to the DEFAULT graph (sparq's
//! `Graph::apply_delta` semantics); named-graph entries are untouched.

use crate::literal::GeoGeometry;
use crate::{geof, vocab};
use geo::{BoundingRect, Intersects};
#[cfg(feature = "topology_index")]
use geo::{PreparedGeometry, Relate};
use geo_types::Point;
use oxrdf::{NamedNode, Term};
use rstar::{Envelope, PointDistance, RTree, RTreeObject, SelectionFunction, AABB};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;
use sparq_core::Graph;

/// Mean-Earth-radius metres per degree of latitude (same sphere as `geo`'s haversine).
const METERS_PER_DEGREE: f64 = std::f64::consts::PI * 6_371_008.8 / 180.0;
/// Half the Earth's great circle: a radius that always covers everything.
const HALF_EARTH_METERS: f64 = std::f64::consts::PI * 6_371_008.8;

/// One indexed geometry: the entity it locates plus the parsed geometry and
/// where it was extracted from.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The entity term answers are reported as: the owning feature via
    /// `geo:hasGeometry` / `geo:hasDefaultGeometry`, else the `geo:asWKT`
    /// subject itself.
    pub entity: Term,
    /// The `geo:asWKT` subject this entry was extracted from (the geometry
    /// node; equal to `entity` when the feature carries `geo:asWKT` directly).
    pub node: Term,
    /// The original `geo:wktLiteral` term (the `geo:asWKT` OBJECT) this entry was
    /// parsed from — the exact `Term` a SPARQL geometry variable binds to. The
    /// spatial-pushdown candidate methods return THIS so the engine can map a
    /// candidate back to a dictionary id by `Term` identity.
    pub literal: Term,
    /// The build-time dictionary `Id` of [`literal`](Self::literal) in the source
    /// [`Graph`]'s `dict` — the id the engine's scanned geometry column holds for a
    /// row bound to this literal. Populated from the `geo:asWKT` OBJECT id at
    /// extraction (see [`GeoIndex::indexed_ids_for`]); valid only against the SAME
    /// dict the index was built from (the index's private `dict_ptr` freshness
    /// token).
    pub literal_id: Id,
    /// The named graph the entry came from; `None` for the default graph.
    pub graph: Option<Term>,
    pub geometry: GeoGeometry,
}

/// An R-tree leaf: the entry's slot plus its precomputed AABB (degree space).
struct TreeItem {
    idx: u32,
    env: AABB<[f64; 2]>,
}

impl RTreeObject for TreeItem {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> AABB<[f64; 2]> {
        self.env
    }
}

impl PointDistance for TreeItem {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        self.env.distance_2(point)
    }
}

/// Selects exactly the leaf with the given slot (descending only through
/// parents that can contain its envelope) — the removal cursor for
/// incremental maintenance.
struct SelectSlot {
    idx: u32,
    env: AABB<[f64; 2]>,
}

impl SelectionFunction<TreeItem> for SelectSlot {
    fn should_unpack_parent(&self, parent: &AABB<[f64; 2]>) -> bool {
        parent.contains_envelope(&self.env)
    }
    fn should_unpack_leaf(&self, leaf: &TreeItem) -> bool {
        leaf.idx == self.idx
    }
}

/// A spatial index over the geometries of a sparq [`Graph`].
pub struct GeoIndex {
    /// Entry slots; `None` slots were removed by a delta and may be reused.
    slots: Vec<Option<Entry>>,
    /// Reusable (tombstoned) slot indices.
    free: Vec<u32>,
    tree: RTree<TreeItem>,
    skipped: usize,
    /// The source `Graph`'s `dict` address captured at [`build`](Self::build) as
    /// an OPAQUE freshness token (NOT a live reference — just the `usize`
    /// address). [`indexed_ids_for`](Self::indexed_ids_for) hands out the id-set
    /// ONLY when the caller's dict address matches this, so a stale index over a
    /// different (reloaded) dict can never mislead the engine's id-level check.
    /// `None` means "never fresh" — the id-level fast path is declined and the
    /// engine uses the per-row `is_indexed` fallback ([`apply_delta`](Self::apply_delta)
    /// sets it to `None` if the graph's dict moved).
    dict_ptr: Option<usize>,
    /// The set of live `Entry::literal_id`s — the id-level indexed universe,
    /// maintained in LOCKSTEP with `slots`/`free` (an id enters when its entry is
    /// inserted, leaves when the LAST entry carrying it is dropped). Valid only
    /// against the dict identified by `dict_ptr`.
    literal_ids: FxHashSet<Id>,
    /// Reference count per live literal id: several entries (one per owning
    /// feature) can share ONE literal id, so a literal id leaves `literal_ids`
    /// only when its last entry is removed. Keeps `literal_ids` exact under the
    /// slotted delta maintenance.
    literal_id_refs: FxHashMap<Id, u32>,
}

/// The `(entity, geometry)` rows extracted from ONE graph's `geo:asWKT`
/// scan, plus the number of skipped literals.
fn extract_graph(graph: &Graph, graph_name: Option<&Term>) -> (Vec<Entry>, usize) {
    // geometry node id -> owning feature ids (geo:hasGeometry / geo:hasDefaultGeometry).
    let mut owners: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for pred in [vocab::HAS_GEOMETRY, vocab::HAS_DEFAULT_GEOMETRY] {
        let pred = NamedNode::new_unchecked(pred);
        if let Some(pattern) = graph.pattern(None, Some(&pred), None) {
            let scan = graph.store().scan(&pattern);
            for row in scan.rows.iter() {
                let [s, _, o] = scan.to_spo(row);
                let features = owners.entry(o).or_default();
                if !features.contains(&s) {
                    features.push(s);
                }
            }
        }
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut skipped = 0usize;
    // Scan both geometry-serialization predicates: geo:asWKT (wktLiteral) and
    // geo:asGML (gmlLiteral). Both flow through the same dispatch.
    for serial_pred in [vocab::AS_WKT, vocab::AS_GML] {
        let pred = NamedNode::new_unchecked(serial_pred);
        let Some(pattern) = graph.pattern(None, Some(&pred), None) else {
            continue;
        };
        let scan = graph.store().scan(&pattern);
        for row in scan.rows.iter() {
            let [s, _, o] = scan.to_spo(row);
            // Only geometry-typed literals (geo:wktLiteral / geo:gmlLiteral);
            // everything else is skipped.
            let lit = match graph.dict().term(o) {
                Term::Literal(l) if crate::is_geometry_datatype(l.datatype().as_str()) => l,
                _ => {
                    skipped += 1;
                    continue;
                }
            };
            let geometry = match crate::parse_geometry_literal(lit.value(), lit.datatype().as_str())
            {
                // The index computes great-circle metres: non-geographic
                // CRSs have no defined conversion, so they are skipped
                // (still usable via the geof:: functions directly). EMPTY
                // geometries have no bounding box (no location) — skipped.
                Ok(g) if g.crs.is_geographic() && g.geometry.bounding_rect().is_some() => g,
                _ => {
                    skipped += 1;
                    continue;
                }
            };
            let node = graph.dict().term(s);
            let literal = Term::Literal(lit.clone());
            // `o` is the geo:asWKT OBJECT id in THIS graph's dict — the id the
            // engine's scanned geometry column binds for a row on this literal.
            // Thread it so the pushdown can decide `is_indexed` at the id level.
            let literal_id = o;
            match owners.get(&s) {
                Some(features) => {
                    for &f in features {
                        entries.push(Entry {
                            entity: graph.dict().term(f),
                            node: node.clone(),
                            literal: literal.clone(),
                            literal_id,
                            graph: graph_name.cloned(),
                            geometry: geometry.clone(),
                        });
                    }
                }
                None => entries.push(Entry {
                    entity: node.clone(),
                    node: node.clone(),
                    literal,
                    literal_id,
                    graph: graph_name.cloned(),
                    geometry,
                }),
            }
        }
    }
    (entries, skipped)
}

/// The degree-space AABB of a (non-empty) geometry.
fn geometry_env(g: &GeoGeometry) -> AABB<[f64; 2]> {
    // Non-empty by construction (empties are skipped during extraction).
    let rect = g.geometry.bounding_rect().expect("non-empty geometry");
    AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y])
}

impl GeoIndex {
    /// Extracts `(entity, geometry)` pairs from the graph — default graph plus
    /// every named graph (see module docs for the RDF shape) — and bulk-loads
    /// the R-tree. A graph with no GeoSPARQL vocabulary yields an empty (but
    /// usable) index.
    pub fn build(graph: &Graph) -> GeoIndex {
        let (mut entries, mut skipped) = extract_graph(graph, None);
        for (name, g) in graph.named() {
            let (more, more_skipped) = extract_graph(g, Some(name));
            entries.extend(more);
            skipped += more_skipped;
        }

        let items: Vec<TreeItem> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| TreeItem { idx: i as u32, env: geometry_env(&e.geometry) })
            .collect();
        // The id-level indexed universe: one ref per entry, one set membership per
        // distinct literal id (features sharing a geometry share its literal id).
        let mut literal_ids: FxHashSet<Id> = FxHashSet::default();
        let mut literal_id_refs: FxHashMap<Id, u32> = FxHashMap::default();
        for e in &entries {
            *literal_id_refs.entry(e.literal_id).or_insert(0) += 1;
            literal_ids.insert(e.literal_id);
        }
        GeoIndex {
            slots: entries.into_iter().map(Some).collect(),
            free: Vec::new(),
            tree: RTree::bulk_load(items),
            skipped,
            // The freshness token: the source dict's address (opaque; no live borrow).
            dict_ptr: Some(std::ptr::from_ref(graph.dict()) as usize),
            literal_ids,
            literal_id_refs,
        }
    }

    /// All indexed entries (one per entity/geometry pair; extraction order on
    /// a fresh build, arbitrary after incremental updates).
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.slots.iter().filter_map(|s| s.as_ref())
    }

    /// Number of indexed entity/geometry pairs.
    pub fn len(&self) -> usize {
        self.tree.size()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.size() == 0
    }

    /// `geo:asWKT` triples that were NOT indexed (cumulative over the build
    /// and any [`apply_delta`](Self::apply_delta) calls): non-wktLiteral
    /// objects, unparseable WKT, or non-geographic CRSs.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// The id-level indexed universe — the set of live `geo:asWKT` literal
    /// dictionary `Id`s this index holds an opinion on — returned ONLY when
    /// `dict_ptr` is the address of the SAME `Graph::dict` this index was built
    /// over (`std::ptr::from_ref(graph.dict()) as usize`), else `None`.
    ///
    /// This is the id-level equivalent of the per-`Term` `is_indexed` check: the
    /// spatial pushdown consults it so a row can be judged "the index has no
    /// opinion on this binding, keep it for the exact `geof:` FILTER" by a pure
    /// `FxHashSet<Id>` lookup on the scanned column, with ZERO per-row `Term`
    /// materialisation. The FRESHNESS gate is load-bearing: an id set only maps
    /// to the right terms against the dict it was extracted from, so a mismatched
    /// address returns `None` and the engine falls back to resolving each row's
    /// term (always correct, just slower).
    pub fn indexed_ids_for(&self, dict_ptr: usize) -> Option<&FxHashSet<Id>> {
        (self.dict_ptr == Some(dict_ptr)).then_some(&self.literal_ids)
    }

    // ---- Incremental maintenance ---------------------------------------------------

    /// Mirrors a [`Graph::apply_delta`] batch into the index: call with the
    /// SAME graph (after its `apply_delta`) and the same insert/delete triple
    /// batches. Every geometry node the batch touches — as a `geo:asWKT`
    /// subject or a `geo:hasGeometry`/`geo:hasDefaultGeometry` object — is
    /// re-extracted from the graph's CURRENT state: its old entries leave the
    /// R-tree, its new ones enter it (both incremental; O(batch × log n)).
    /// Non-geo triples in the batch are ignored, so it is safe (just a no-op)
    /// to forward every update batch. Deltas affect default-graph entries
    /// only, matching `Graph::apply_delta`.
    pub fn apply_delta(&mut self, graph: &Graph, inserts: &[[Term; 3]], deletes: &[[Term; 3]]) {
        // Freshness: the id-level indexed universe is only meaningful against the
        // dict it was built over. `Graph::apply_delta` mutates the graph IN PLACE
        // (the `Dict` field's address is stable across a delta), so a matching
        // address means the maintained `literal_ids` stay authoritative and the
        // engine's id-level fast path remains sound; a MISMATCH (a different dict
        // was passed) invalidates freshness so the engine falls back to the per-row
        // check — never a stale id-set.
        let now_ptr = std::ptr::from_ref(graph.dict()) as usize;
        if self.dict_ptr != Some(now_ptr) {
            self.dict_ptr = None;
        }
        // 1. The geometry nodes whose extracted state may have changed.
        let mut nodes: Vec<Term> = Vec::new();
        let mut push_unique = |t: &Term| {
            if !nodes.contains(t) {
                nodes.push(t.clone());
            }
        };
        for [s, p, o] in inserts.iter().chain(deletes) {
            match p {
                // Both geometry-serialization predicates touch the subject node.
                Term::NamedNode(p) if p.as_str() == vocab::AS_WKT || p.as_str() == vocab::AS_GML => {
                    push_unique(s)
                }
                Term::NamedNode(p)
                    if p.as_str() == vocab::HAS_GEOMETRY
                        || p.as_str() == vocab::HAS_DEFAULT_GEOMETRY =>
                {
                    push_unique(o)
                }
                _ => {}
            }
        }
        if nodes.is_empty() {
            return;
        }

        // 2. Drop every default-graph entry derived from an affected node.
        for idx in 0..self.slots.len() as u32 {
            let matches = self.slots[idx as usize]
                .as_ref()
                .is_some_and(|e| e.graph.is_none() && nodes.contains(&e.node));
            if matches {
                let entry = self.slots[idx as usize].take().expect("checked Some");
                // Drop this entry's hold on its literal id; the id leaves the
                // indexed universe only when its LAST entry is removed.
                if let Some(refs) = self.literal_id_refs.get_mut(&entry.literal_id) {
                    *refs -= 1;
                    if *refs == 0 {
                        self.literal_id_refs.remove(&entry.literal_id);
                        self.literal_ids.remove(&entry.literal_id);
                    }
                }
                let removed = self
                    .tree
                    .remove_with_selection_function(SelectSlot {
                        idx,
                        env: geometry_env(&entry.geometry),
                    })
                    .is_some();
                debug_assert!(removed, "tree leaf must exist for a live slot");
                self.free.push(idx);
            }
        }

        // 3. Re-extract the affected nodes from the graph's current state.
        for node in &nodes {
            // The node's owning features, post-delta.
            let mut owners: Vec<Term> = Vec::new();
            for pred in [vocab::HAS_GEOMETRY, vocab::HAS_DEFAULT_GEOMETRY] {
                let pred = NamedNode::new_unchecked(pred);
                if let Some(pattern) = graph.pattern(None, Some(&pred), Some(node)) {
                    let scan = graph.store().scan(&pattern);
                    for row in scan.rows.iter() {
                        let [s, _, _] = scan.to_spo(row);
                        let feature = graph.dict().term(s);
                        if !owners.contains(&feature) {
                            owners.push(feature);
                        }
                    }
                }
            }
            // The node's geometry literals, post-delta, over BOTH serialization
            // predicates (same skip rules as build).
            for serial_pred in [vocab::AS_WKT, vocab::AS_GML] {
                let pred = NamedNode::new_unchecked(serial_pred);
                let Some(pattern) = graph.pattern(Some(node), Some(&pred), None) else {
                    continue;
                };
                let scan = graph.store().scan(&pattern);
                for row in scan.rows.iter() {
                    let [_, _, o] = scan.to_spo(row);
                    let lit = match graph.dict().term(o) {
                        Term::Literal(l) if crate::is_geometry_datatype(l.datatype().as_str()) => l,
                        _ => {
                            self.skipped += 1;
                            continue;
                        }
                    };
                    let geometry =
                        match crate::parse_geometry_literal(lit.value(), lit.datatype().as_str()) {
                            Ok(g)
                                if g.crs.is_geographic()
                                    && g.geometry.bounding_rect().is_some() =>
                            {
                                g
                            }
                            _ => {
                                self.skipped += 1;
                                continue;
                            }
                        };
                    let literal = Term::Literal(lit.clone());
                    // Post-delta OBJECT id in the (same) graph's dict.
                    let literal_id = o;
                    if owners.is_empty() {
                        self.insert_entry(Entry {
                            entity: node.clone(),
                            node: node.clone(),
                            literal,
                            literal_id,
                            graph: None,
                            geometry,
                        });
                    } else {
                        for feature in &owners {
                            self.insert_entry(Entry {
                                entity: feature.clone(),
                                node: node.clone(),
                                literal: literal.clone(),
                                literal_id,
                                graph: None,
                                geometry: geometry.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    /// Adds one entry to the slot array and the R-tree (incremental insert).
    fn insert_entry(&mut self, entry: Entry) {
        let env = geometry_env(&entry.geometry);
        // Keep the id-level indexed universe in lockstep: bump this literal id's
        // refcount, adding it to the set on its first live entry.
        *self.literal_id_refs.entry(entry.literal_id).or_insert(0) += 1;
        self.literal_ids.insert(entry.literal_id);
        let idx = match self.free.pop() {
            Some(idx) => {
                self.slots[idx as usize] = Some(entry);
                idx
            }
            None => {
                self.slots.push(Some(entry));
                (self.slots.len() - 1) as u32
            }
        };
        self.tree.insert(TreeItem { idx, env });
    }

    // ---- Queries --------------------------------------------------------------------

    /// The pole-safe long/lat window(s) guaranteed to contain the great-circle
    /// ball of `meters` around `center`. A ball crossing the ANTIMERIDIAN
    /// yields two windows (one per side of ±180°); otherwise one.
    fn ball_windows(center: Point<f64>, meters: f64) -> Vec<AABB<[f64; 2]>> {
        let dlat = meters / METERS_PER_DEGREE;
        let lat_min = (center.y() - dlat).max(-90.0);
        let lat_max = (center.y() + dlat).min(90.0);
        // Longitude degrees shrink with cos(lat): size the window for the
        // WORST latitude it spans so the window is a superset of the ball.
        let max_abs_lat = lat_min.abs().max(lat_max.abs()).to_radians();
        if lat_min <= -90.0 + 1e-12
            || lat_max >= 90.0 - 1e-12
            || max_abs_lat.cos() * METERS_PER_DEGREE * 180.0 <= meters
        {
            // The ball encloses a pole (or the whole longitude band): every
            // longitude is inside.
            return vec![AABB::from_corners([-180.0, lat_min], [180.0, lat_max])];
        }
        // Normalise the center into [-180, 180] so the wrap arithmetic below
        // is exact for callers passing e.g. 181°.
        let cx = (center.x() + 180.0).rem_euclid(360.0) - 180.0;
        let dlon = meters / (METERS_PER_DEGREE * max_abs_lat.cos());
        let (lon_min, lon_max) = (cx - dlon, cx + dlon);
        if lon_min < -180.0 {
            // Crosses the antimeridian westward: [-180, lon_max] + the wrapped
            // remainder on the +180 side.
            vec![
                AABB::from_corners([-180.0, lat_min], [lon_max, lat_max]),
                AABB::from_corners([lon_min + 360.0, lat_min], [180.0, lat_max]),
            ]
        } else if lon_max > 180.0 {
            // Crosses eastward: [lon_min, 180] + the wrapped remainder.
            vec![
                AABB::from_corners([lon_min, lat_min], [180.0, lat_max]),
                AABB::from_corners([-180.0, lat_min], [lon_max - 360.0, lat_max]),
            ]
        } else {
            vec![AABB::from_corners([lon_min, lat_min], [lon_max, lat_max])]
        }
    }

    /// All entities whose geometry lies within `meters` great-circle metres of
    /// `center` (a CRS84 long/lat point), nearest first, truncated to `limit`
    /// (when given). Distance to an extended geometry is to its (spherical)
    /// closest point. Balls crossing the antimeridian are handled (split into
    /// two windows internally).
    pub fn within_distance(
        &self,
        center: Point<f64>,
        meters: f64,
        limit: Option<usize>,
    ) -> Vec<(&Term, f64)> {
        let windows = Self::ball_windows(center, meters);
        // An entry's AABB can intersect both halves of a split window: dedupe.
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        let mut hits: Vec<(&Term, f64)> = Vec::new();
        for window in &windows {
            // rstar 0.13 takes the envelope by value (AABB is Copy); deref the &window.
            for item in self.tree.locate_in_envelope_intersecting(*window) {
                if windows.len() > 1 && !seen.insert(item.idx) {
                    continue;
                }
                let e = self.slots[item.idx as usize].as_ref().expect("live slot");
                let Ok(d) = geof::point_to_geometry_meters(center, &e.geometry.geometry) else {
                    continue;
                };
                if d <= meters {
                    hits.push((&e.entity, d));
                }
            }
        }
        hits.sort_by(|a, b| a.1.total_cmp(&b.1));
        if let Some(k) = limit {
            hits.truncate(k);
        }
        hits
    }

    /// Convenience: [`within_distance`](Self::within_distance) from a wktLiteral
    /// point lexical form. sq-bif.19
    pub fn within_distance_wkt(
        &self,
        center_wkt: &str,
        meters: f64,
        limit: Option<usize>,
    ) -> Result<Vec<(&Term, f64)>, crate::GeoError> {
        let center = crate::parse_wkt_literal(center_wkt)?;
        let geo_types::Geometry::Point(center) = center.geometry else {
            return Err(crate::GeoError::Unsupported(
                "within_distance_wkt center must be a Point geometry".to_string(),
            ));
        };
        Ok(self.within_distance(center, meters, limit))
    }

    /// The `k` entities nearest to `center` (great-circle metres, nearest
    /// first). Exact under the same metric as
    /// [`within_distance`](Self::within_distance): an
    /// expanding-radius search (×4 per round) that stops once `k` results fit
    /// inside the current radius or the radius covers the whole Earth.
    pub fn nearest(&self, center: Point<f64>, k: usize) -> Vec<(&Term, f64)> {
        if k == 0 || self.tree.size() == 0 {
            return Vec::new();
        }
        // Seed the radius from the data extent: a k/n area share of the
        // bounding box diagonal (clamped) keeps the first window useful for
        // both dense city-scale and sparse global data.
        let root = self.tree.root().envelope();
        let diag_deg = {
            let lo = root.lower();
            let hi = root.upper();
            ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2)).sqrt()
        };
        let share = (k as f64 / self.tree.size().max(1) as f64).sqrt();
        let mut radius = (diag_deg * share * METERS_PER_DEGREE).clamp(1.0, HALF_EARTH_METERS);
        loop {
            let hits = self.within_distance(center, radius, Some(k));
            // Complete when k results fit strictly inside the radius (the
            // (k+1)-th nearest cannot be closer than the window bound), or
            // nothing can be further away.
            if hits.len() == k || radius >= HALF_EARTH_METERS {
                return hits;
            }
            radius = (radius * 4.0).min(HALF_EARTH_METERS);
        }
    }

    /// All entities whose geometry intersects `geometry` (simple-features
    /// `sfIntersects`, evaluated in long/lat degree space). The argument must
    /// be in a geographic CRS, matching the indexed geometries; its
    /// coordinates are taken as written (no antimeridian wrapping — the
    /// refinement itself is planar in degree space).
    pub fn intersects(&self, geometry: &GeoGeometry) -> Vec<&Term> {
        let rect = match geometry.geometry.bounding_rect() {
            Some(r) => r,
            None => return Vec::new(),
        };
        if !geometry.crs.is_geographic() {
            return Vec::new();
        }
        let window = AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y]);
        self.tree
            // rstar 0.13 takes the envelope by value (AABB is Copy).
            .locate_in_envelope_intersecting(window)
            .filter_map(|item| {
                let e = self.slots[item.idx as usize].as_ref().expect("live slot");
                e.geometry.geometry.intersects(&geometry.geometry).then_some(&e.entity)
            })
            .collect()
    }

    /// Convenience: [`intersects`](Self::intersects) from a wktLiteral lexical form.
    pub fn intersects_wkt(&self, wkt_literal: &str) -> Result<Vec<&Term>, crate::GeoError> {
        Ok(self.intersects(&crate::parse_wkt_literal(wkt_literal)?))
    }

    // ---- Spatial-pushdown candidates (sq-mg9) --------------------------------------
    //
    // These return the original `geo:wktLiteral` TERMS (`Entry::literal`), deduped, as a
    // candidate SUPERSET for a `geof:` FILTER on a geometry variable. The SPARQL engine
    // maps them back to dictionary ids by term identity and pre-restricts the binding rows
    // before the exact `geof:` refinement runs — so a candidate set that is a superset of
    // the true matches is necessary and sufficient for correctness (false positives are
    // removed by the residual FILTER; false negatives would be a bug).

    /// The distinct geometry LITERALS whose geometry lies within `meters` great-circle
    /// metres of `center` — a candidate superset for `geof:distance(?g, center) <(=) r`.
    /// Reuses the exact metric refinement of [`within_distance`](Self::within_distance),
    /// so the only false positives are from MULTIPLE literals sharing one entity (the set
    /// is keyed on the literal, not the entity).
    pub fn within_distance_literals(&self, center: Point<f64>, meters: f64) -> Vec<Term> {
        let windows = Self::ball_windows(center, meters);
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        let mut out: Vec<Term> = Vec::new();
        for window in &windows {
            // rstar 0.13 takes the envelope by value (AABB is Copy); deref the &window.
            for item in self.tree.locate_in_envelope_intersecting(*window) {
                if !seen.insert(item.idx) {
                    continue;
                }
                let e = self.slots[item.idx as usize].as_ref().expect("live slot");
                if let Ok(d) = geof::point_to_geometry_meters(center, &e.geometry.geometry) {
                    if d <= meters {
                        out.push(e.literal.clone());
                    }
                }
            }
        }
        dedupe(out)
    }

    /// The distinct geometry LITERALS whose bounding box intersects `geometry`'s bounding
    /// box — a pure R-tree window scan (NO exact refinement), a candidate superset for
    /// BOTH `geof:sfIntersects(?g, geometry)` and `geof:sfWithin(?g, geometry)` (A within B
    /// ⟹ A intersects B ⟹ their AABBs intersect). The engine's residual `geof:` FILTER
    /// does the exact check. Empty for a non-geographic / empty argument.
    pub fn bbox_candidate_literals(&self, geometry: &GeoGeometry) -> Vec<Term> {
        let rect = match geometry.geometry.bounding_rect() {
            Some(r) => r,
            None => return Vec::new(),
        };
        if !geometry.crs.is_geographic() {
            return Vec::new();
        }
        let window = AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y]);
        let out: Vec<Term> = self
            .tree
            // rstar 0.13 takes the envelope by value (AABB is Copy).
            .locate_in_envelope_intersecting(window)
            .map(|item| self.slots[item.idx as usize].as_ref().expect("live slot").literal.clone())
            .collect();
        dedupe(out)
    }

    /// The distinct indexed geometry literals that satisfy
    /// `geof:sfWithin(literal, region)` exactly.
    ///
    /// This opt-in topology-index path window-scans the region's bounding box,
    /// prepares the constant region once, and applies the DE-9IM relation to
    /// each candidate. Unlike [`bbox_candidate_literals`](Self::bbox_candidate_literals),
    /// the returned set is exact rather than a candidate superset. Empty and
    /// non-geographic regions return an empty set.
    #[cfg(feature = "topology_index")]
    #[cfg_attr(docsrs, doc(cfg(feature = "topology_index")))]
    pub fn within_region_literals(&self, region: &GeoGeometry) -> Vec<Term> {
        let rect = match region.geometry.bounding_rect() {
            Some(rect) => rect,
            None => return Vec::new(),
        };
        if !region.crs.is_geographic() {
            return Vec::new();
        }

        // sq-jrdds: prepare the CONSTANT side once per scan. Each
        // candidate still contributes its own geometry graph, but never rebuilds
        // the region's topology graph.
        let prepared_region = PreparedGeometry::from(region.geometry.clone());
        let window = AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y]);
        let out = self
            .tree
            .locate_in_envelope_intersecting(window)
            .filter_map(|item| {
                let entry = self.slots[item.idx as usize].as_ref().expect("live slot");
                entry
                    .geometry
                    .geometry
                    .relate(&prepared_region)
                    .is_within()
                    .then(|| entry.literal.clone())
            })
            .collect();
        dedupe(out)
    }

    /// The distinct indexed geometry literals that satisfy
    /// `geof:sfContains(region, literal)` exactly.
    ///
    /// This is the constant-first orientation of
    /// [`within_region_literals`](Self::within_region_literals): Simple Features
    /// defines `contains(a, b)` as the converse of `within(b, a)`, so both
    /// methods return the same exact literal set.
    #[cfg(feature = "topology_index")]
    #[cfg_attr(docsrs, doc(cfg(feature = "topology_index")))]
    pub fn contains_region_literals(&self, region: &GeoGeometry) -> Vec<Term> {
        self.within_region_literals(region)
    }
}

/// Order-preserving dedupe of candidate terms (the same literal can be indexed under
/// several entities; the engine only needs each distinct binding once).
fn dedupe(mut v: Vec<Term>) -> Vec<Term> {
    let mut seen: FxHashSet<Term> = FxHashSet::default();
    v.retain(|t| seen.insert(t.clone()));
    v
}

#[cfg(test)]
mod indexed_ids_tests {
    // sq-7jt80 — direct unit tests for the id-level indexed universe
    // (`indexed_ids_for`). This set is the id-level equivalent of the per-`Term`
    // `is_indexed` check the spatial pushdown uses; correctness here is
    // ANSWER-SAFETY (a wrong verdict would silently drop or wrongly keep rows).
    use super::*;

    /// Build a small feature-shaped graph: one indexed geometry per feature.
    fn graph() -> Graph {
        Graph::load_str(
            r#"@prefix geo: <http://www.opengis.net/ont/geosparql#> .
               @prefix ex:  <http://ex/> .
               ex:a geo:hasGeometry ex:ga . ex:ga geo:asWKT "POINT(0 0)"^^geo:wktLiteral .
               ex:b geo:hasGeometry ex:gb . ex:gb geo:asWKT "POINT(1 1)"^^geo:wktLiteral ."#,
            "turtle",
        )
        .unwrap()
    }

    #[test]
    fn indexed_ids_for_returns_the_live_literal_ids_on_a_fresh_dict_match() {
        let g = graph();
        let idx = GeoIndex::build(&g);
        let dict_ptr = std::ptr::from_ref(g.dict()) as usize;
        let ids = idx.indexed_ids_for(dict_ptr).expect("fresh dict must match");
        // Exactly the two geo:asWKT literal ids, and each is the id `id_of` resolves
        // for the corresponding literal Term the index reports as indexed.
        assert_eq!(ids.len(), 2, "two distinct geometry literals");
        for e in idx.entries() {
            assert!(ids.contains(&e.literal_id), "every entry's literal id is in the set");
            // The id-level and Term-level views agree: the id is the one the graph's
            // dict resolves for the same literal Term.
            assert_eq!(g.id_of(&e.literal), Some(e.literal_id));
        }
    }

    #[test]
    fn indexed_ids_for_declines_on_a_dict_pointer_mismatch() {
        // A DIFFERENT dict address is NOT the one the ids were extracted against:
        // the freshness gate must return None so the engine uses the per-row path.
        let g = graph();
        let idx = GeoIndex::build(&g);
        let other = Graph::load_str("", "ntriples").unwrap();
        let other_ptr = std::ptr::from_ref(other.dict()) as usize;
        assert!(
            idx.indexed_ids_for(other_ptr).is_none(),
            "a mismatched dict address must decline the id-level universe"
        );
        // A plainly-bogus address also declines.
        assert!(idx.indexed_ids_for(0).is_none());
    }

    #[test]
    fn shared_geometry_id_is_refcounted_across_owning_features() {
        // Two features owning the SAME geometry node share ONE literal id: the set
        // holds it once, and deleting one owner must NOT evict it (the other still
        // holds it). Exercises the literal-id refcount in insert/remove lockstep.
        let mut g = Graph::load_str(
            r#"@prefix geo: <http://www.opengis.net/ont/geosparql#> .
               @prefix ex:  <http://ex/> .
               ex:a geo:hasGeometry ex:g . ex:b geo:hasGeometry ex:g .
               ex:g geo:asWKT "POINT(0 0)"^^geo:wktLiteral ."#,
            "turtle",
        )
        .unwrap();
        let mut idx = GeoIndex::build(&g);
        // Two entries (a, b) but ONE distinct literal id.
        assert_eq!(idx.entries().count(), 2);
        let dict_ptr = std::ptr::from_ref(g.dict()) as usize;
        assert_eq!(idx.indexed_ids_for(dict_ptr).unwrap().len(), 1);

        // Remove ONE ownership edge; the geometry (and its literal id) survives.
        let iri = |s: &str| Term::NamedNode(NamedNode::new_unchecked(s.to_string()));
        let del = [[iri("http://ex/a"), iri(vocab::HAS_GEOMETRY), iri("http://ex/g")]];
        g.apply_delta(&[], &del).unwrap();
        idx.apply_delta(&g, &[], &del);
        let dict_ptr = std::ptr::from_ref(g.dict()) as usize;
        // One entry (b) remains; the literal id is still present exactly once.
        assert_eq!(idx.entries().count(), 1);
        assert_eq!(idx.indexed_ids_for(dict_ptr).unwrap().len(), 1);
        assert_eq!(*idx.indexed_ids_for(dict_ptr).unwrap(), GeoIndex::build(&g).indexed_ids_for(dict_ptr).unwrap().clone());
    }
}
