---
name: geosparql
description: "Use when adding GeoSPARQL spatial support to the sparq RDF/SPARQL engine — parsing geo:wktLiteral and geo:gmlLiteral (GML Simple-Features) geometries, calling geof: functions (distance, metricArea/metricLength/metricPerimeter/centroid, bounding coordinates, isEmpty, simplify, sf*/eh*/rcc8* DE-9IM relations, geometry/set operations, getSRID) inside SPARQL FILTER/BIND/SELECT via the sparq-engine extension-function registry, or building an R-tree GeoIndex over a Graph for within_distance / nearest / intersects queries. Crate: sparq-geo."
---

# sparq-geosparql

`sparq-geo` is the OPT-IN GeoSPARQL 1.0/1.1 **core** for the sparq engine: it parses `geo:wktLiteral` and `geo:gmlLiteral` (the GML Simple-Features geometry profile) lexical forms, evaluates the `geof:` spatial functions, packages those functions as a `sparq_engine::FunctionRegistry` so they run inside real SPARQL `FILTER`/`BIND`/`SELECT`, and builds an R-tree `GeoIndex` over a `sparq_core::Graph` for distance/nearest/intersection queries. It is a separate crate so the core engine and the wasm build carry zero geometry code — you engage spatial support only by depending on `sparq-geo`.

## Quickstart

`Cargo.toml`:
```toml
[dependencies]
sparq-geo    = { path = "../sparq-geo" }   # default features include `engine`
sparq-core   = { path = "../sparq-core" }
sparq-engine = { path = "../sparq-engine" }
geo-types    = "0.7"                        # only if you use GeoIndex (Point<f64>)
```

Run `geof:` functions inside a SPARQL query:
```rust
use sparq_core::Graph;
use sparq_engine::query_with_functions;
use sparq_geo::geof_registry;

let g = Graph::load_str(r#"
    @prefix geo: <http://www.opengis.net/ont/geosparql#> .
    <http://ex/london> <http://ex/loc> "POINT(-0.1278 51.5074)"^^geo:wktLiteral .
    <http://ex/paris>  <http://ex/loc> "POINT(2.3522 48.8566)"^^geo:wktLiteral .
"#, "turtle").unwrap();

let reg = geof_registry();               // 45 functions; clone-cheap, Send + Sync
let r = query_with_functions(&g,
    "PREFIX geof: <http://www.opengis.net/def/function/geosparql/>
     PREFIX uom:  <http://www.opengis.net/def/uom/OGC/1.0/>
     SELECT ?a ?b WHERE {
       ?a <http://ex/loc> ?ga . ?b <http://ex/loc> ?gb .
       FILTER(STR(?a) < STR(?b) && geof:distance(?ga, ?gb, uom:kilometre) < 400)
     }", &reg).unwrap();
assert_eq!(r.len(), 1);                   // London–Paris ≈ 343.6 km
```

`POINT` / WKT coordinates are **longitude latitude** (CRS84, the GeoSPARQL default). A leading `<CRS-IRI>` before the WKT selects a CRS; `EPSG:4326` literals are LAT/LONG and are axis-swapped to internal long/lat on parse and back on output.

## Key APIs

```rust
// --- Geometry literals (always available) ---
pub fn parse_wkt_literal(lex: &str) -> Result<GeoGeometry, GeoError>;   // geo:wktLiteral
pub fn parse_gml_literal(lex: &str) -> Result<GeoGeometry, GeoError>;   // geo:gmlLiteral (GML-SF)
pub fn parse_geometry_literal(value: &str, datatype: &str) -> Result<GeoGeometry, GeoError>; // dispatch by datatype
pub fn is_geometry_datatype(datatype: &str) -> bool;   // geo:wktLiteral | geo:gmlLiteral
pub struct GeoGeometry { pub crs: Crs, pub geometry: geo_types::Geometry<f64> }
impl GeoGeometry { pub fn new(g: Geometry<f64>) -> Self;        // CRS84
                   pub fn to_wkt_literal(&self) -> String;      // lexical form
                   pub fn to_gml_literal(&self) -> String;      // GML 3 lexical form
                   pub fn metadata(&self) -> GeometryMetadata; }// OGC R9 geo: metadata props
pub enum Crs { Crs84, Epsg4326, Other(String) }                 // .iri(), .is_geographic()

// --- geo:Geometry metadata properties (OGC R9; always available) ---
// geo:dimension / coordinateDimension / spatialDimension / isEmpty / isSimple / hasSerialization.
pub struct GeometryMetadata { pub dimension: Option<u8>,          // topological 0/1/2 (None = empty)
    pub coordinate_dimension: Option<u8>, pub spatial_dimension: Option<u8>,  // 2 in this 2-D profile
    pub is_empty: bool, pub is_simple: bool, pub has_serialization: String }  // WKT lexical form
// sparq_geo::metadata::lex::{dimension,coordinate_dimension,spatial_dimension,is_empty,is_simple,
//   has_serialization,metadata}(value, datatype) — the lexical mirror (str + datatype IRI in).

// --- geof: as the SPARQL extension-function registry (default-on `engine` feature) ---
pub fn geof_registry() -> sparq_engine::FunctionRegistry;
// drive with:  sparq_engine::query_with_functions(&graph, sparql, &reg) -> Result<QueryResult, String>
//   or scope another entry point: sparq_engine::with_functions(&reg, || sparq_engine::ask(&g, q))

// --- GeoSPARQL query-rewrite extension: topology PROPERTY forms ---
// OPT-IN `geosparql_rewrite` feature (OFF by default; implies `engine`). The whole
// surface below compiles out of a default build, so default SPARQL behaviour is
// unchanged. Expands `?a geo:sfWithin ?b` (sf*/eh*/rcc8*) triple patterns into
// `(geo:hasDefaultGeometry|geo:hasGeometry)/geo:asWKT` geometry-resolution joins +
// the matching `geof:` FILTER. Run the returned PreparedQuery UNDER geof_registry().
// Opt-in entry point only — the standard sparq_engine entry points still match a
// geo:sfWithin predicate as an ordinary asserted triple (W3C conformance unaffected).
// Conformance: tests/ogc_query_rewrite_ratchet.rs pins the measured property-form
// pass count (the OGC `/conf/query-rewrite-extension` class) in the central scoreboard.
pub fn geosparql_rewrite(sparql: &str) -> Result<sparq_engine::PreparedQuery, String>;
pub fn rewrite_query(q: spargebra::Query) -> spargebra::Query;   // algebra-level form
pub fn is_topology_property(iri: &str) -> bool;                  // recognises the geo: topology IRIs

// --- geof: as plain Rust over GeoGeometry (always available) ---
pub fn distance(a: &GeoGeometry, b: &GeoGeometry, unit: Unit) -> Result<f64, GeoError>;
pub fn metric_area(g: &GeoGeometry) -> Result<f64, GeoError>;       // square metres
pub fn metric_length(g: &GeoGeometry) -> Result<f64, GeoError>;     // metres
pub fn metric_perimeter(g: &GeoGeometry) -> Result<f64, GeoError>;  // metres
pub fn centroid(g: &GeoGeometry) -> Result<GeoGeometry, GeoError>;  // POINT, same CRS
pub fn max_x|min_x|max_y|min_y(g: &GeoGeometry) -> Result<f64, GeoError>; // envelope coordinate, stored CRS
pub fn is_empty(g: &GeoGeometry) -> Result<bool, GeoError>;         // unit-free geometry predicate
pub fn simplify(g: &GeoGeometry, tolerance: f64) -> Result<GeoGeometry, GeoError>; // RDP, same CRS
pub fn sf_within(a, b) -> Result<bool, GeoError>;   // + sf_equals/disjoint/intersects/touches/
                                                    //   crosses/contains/overlaps, eh_*, rcc8_*
pub fn relate(a, b, pattern: &str) -> Result<bool, GeoError>;   // generic 9-char DE-9IM
pub fn envelope|boundary|convex_hull(g: &GeoGeometry) -> Result<GeoGeometry, GeoError>;
pub fn buffer(g: &GeoGeometry, radius: f64, unit: Unit) -> Result<GeoGeometry, GeoError>;
pub fn intersection|union|difference|sym_difference(a, b) -> Result<GeoGeometry, GeoError>;
pub enum Unit { Metre, Kilometre, Mile, Degree, Radian }        // Unit::from_iri(uom_iri)
// geof::lex::* provides wktLiteral-string helpers for geometry operations and relations.

// --- GeoIndex: R-tree over a Graph (always available) ---
pub fn GeoIndex::build(graph: &sparq_core::Graph) -> GeoIndex;
pub fn within_distance(&self, center: Point<f64>, meters: f64, limit: Option<usize>) -> Vec<(&Term, f64)>; // + within_distance_wkt(&str, meters, limit) [GPT-5.6]
pub fn nearest(&self, center: Point<f64>, k: usize) -> Vec<(&Term, f64)>;
pub fn intersects(&self, g: &GeoGeometry) -> Vec<&Term>;        // + intersects_wkt(&str)
// `topology_index` feature (OFF by default): exact DE-9IM matches after an R-tree window scan.
pub fn within_region_literals(&self, region: &GeoGeometry) -> Vec<Term>;   // sfWithin(literal, region)
pub fn contains_region_literals(&self, region: &GeoGeometry) -> Vec<Term>; // sfContains(region, literal)
pub fn apply_delta(&mut self, graph: &Graph, inserts: &[[Term;3]], deletes: &[[Term;3]]);
pub fn entries(&self) -> impl Iterator<Item=&Entry>;  // len() / is_empty() / skipped()
```

Function IRIs are all under `http://www.opengis.net/def/function/geosparql/`. Result types in SPARQL: `geof:distance` / `metricArea` / `metricLength` / `metricPerimeter` / `maxX` / `minX` / `maxY` / `minY` → `xsd:double`; `isEmpty` and the relation families → `xsd:boolean`; `envelope`/`boundary`/`centroid`/`convexHull`/`simplify`/`buffer`/the four set ops → `geo:wktLiteral`; `getSRID` → `xsd:anyURI`. [GPT-5.6] sq-lc2io

The OPT-IN `geof_accessors` feature (OFF by default; the default `geof_registry()` byte-set is unchanged) adds the GeoSPARQL 1.1 §8.5 non-topological accessor functions — `geof:dimension` / `geof:coordinateDimension` / `geof:spatialDimension` → `xsd:integer`, `geof:isSimple` → `xsd:boolean`, `geof:geometryType` → the OGC Simple-Features geometry-class IRI (`sf:Point` etc.) — each delegating to the pure `sparq_geo::metadata` accessors; undefined cases (an empty geometry's dimensions, an unmapped collection's `sf:` class) return a `GeoError::Unsupported`-mapped per-row expression error rather than a fabricated value. [FABLE-5] sq-lsp7k

## Common recipes

**Spatial join — which point is within which polygon (SPARQL):**
```rust
let r = query_with_functions(&g,
  "PREFIX geof: <http://www.opengis.net/def/function/geosparql/>
   SELECT ?city ?region WHERE {
     ?city <http://ex/loc> ?pt . ?region <http://ex/area> ?poly .
     FILTER(geof:sfWithin(?pt, ?poly)) }", &geof_registry()).unwrap();
```

**Topology PROPERTY form via the query-rewrite extension** (opt-in `geosparql_rewrite` feature) — write the relation as a triple, not a FILTER; the rewrite resolves each feature's default geometry and applies the matching `geof:`:
```rust
let prepared = sparq_geo::geosparql_rewrite(
  "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
   SELECT ?city WHERE { ?city geo:sfWithin <http://ex/region> }").unwrap();
let reg = geof_registry();
let r = sparq_engine::with_functions(&reg, || sparq_engine::query_prepared(&g, &prepared)).unwrap();
// Needs ?city (geo:hasDefaultGeometry|geo:hasGeometry)/geo:asWKT <wktLiteral> in the data.
```

**Chain geometry-producing + relation functions in one expression:** `geof:envelope` / `geof:buffer` return a `geo:wktLiteral` that feeds straight back into another `geof:` call:
```sparql
FILTER(geof:sfWithin(?there, geof:buffer(?here, 400, uom:kilometre)))   # 400 km buffer
FILTER(geof:sfContains(geof:envelope(?poly), ?pt))
```

**Build an R-tree index and run k-NN / radius / intersection queries:**
```rust
use geo_types::Point;
use sparq_geo::{GeoIndex, parse_wkt_literal};

let index = GeoIndex::build(&graph);              // scans geo:asWKT + geo:asGML (+ hasGeometry ownership)
let near  = index.nearest(Point::new(2.35, 48.86), 5);            // Vec<(&Term, f64 metres)>
let ball  = index.within_distance(Point::new(2.35, 48.86), 50_000.0, Some(20)); // ≤50 km, top 20
let hits  = index.intersects(&parse_wkt_literal(
                "POLYGON((-1 48, 3 48, 3 49, -1 49, -1 48))").unwrap()); // Vec<&Term>
// Center is a CRS84 long/lat point; distances are great-circle metres, nearest first.
// Antimeridian-crossing balls are handled. Empty/non-geographic literals are skipped() not indexed.
```

**Keep the index in sync with graph updates (no rebuild):** apply the SAME batch to the graph, then to the index:
```rust
graph.apply_delta(&inserts, &deletes).unwrap();   // sparq_core::Graph, &[[Term;3]]
index.apply_delta(&graph, &inserts, &deletes);     // O(batch × log n); non-geo triples are a no-op
```

**Call geof: directly as Rust (no SPARQL engine, works with `--no-default-features`):**
```rust
use sparq_geo::{geof, geof::Unit, parse_wkt_literal};
let a = parse_wkt_literal("POINT(-0.1278 51.5074)").unwrap();
let b = parse_wkt_literal("POINT(2.3522 48.8566)").unwrap();
let km = geof::distance(&a, &b, Unit::Kilometre).unwrap();        // ≈ 343.6
let inside = geof::sf_within(&b, &parse_wkt_literal(
                "POLYGON((-1 42.5,7 42.5,7 51,-1 51,-1 42.5))").unwrap()).unwrap();
let eastern_edge = geof::max_x(&b).unwrap(); // coordinate in b's stored CRS
let empty = geof::is_empty(&b).unwrap();      // false; no CRS/unit decision involved
```

**Parse or serialise a GML literal (or dispatch parsing by datatype):** GML rides the same `GeoGeometry` as WKT, so it works everywhere a parsed geometry does. `to_gml_literal` emits the GML 3 Simple Features form and preserves CRS axis order:
```rust
use sparq_geo::{geof, geof::Unit, parse_gml_literal, parse_geometry_literal, vocab};
let london = parse_gml_literal(
    "<gml:Point><gml:pos>-0.1278 51.5074</gml:pos></gml:Point>").unwrap();
let paris  = parse_geometry_literal(           // dispatch by datatype IRI
    "<gml:Point><gml:pos>2.3522 48.8566</gml:pos></gml:Point>", vocab::GML_LITERAL).unwrap();
let km = geof::distance(&london, &paris, Unit::Kilometre).unwrap();   // ≈ 343.6 (== the WKT twin)
let london_gml = london.to_gml_literal();       // valid geo:gmlLiteral lexical form
assert_eq!(parse_gml_literal(&london_gml).unwrap(), london);
```
In SPARQL, a `"…"^^geo:gmlLiteral` object is accepted by every `geof:` function exactly like a `geo:wktLiteral`, and attaching geometry via `geo:asGML` makes it indexable by `GeoIndex::build` alongside `geo:asWKT`.

**Reproject a projected literal into CRS84 (opt-in `reproject` feature):**
```rust
// Cargo: sparq-geo = { path = "...", features = ["reproject"] }
use sparq_geo::reproject::{to_crs84, to_crs84_lex};
let crs84 = to_crs84_lex(
  "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(530000 180000)").unwrap();
// British National Grid → CRS84 long/lat; then index / metric-distance it.
```

## Gotchas / feature flags / prerequisites

- **`engine` feature (default ON)** pulls in `sparq-engine` and exposes `geof_registry()`. Disable default features (`default-features = false`) for the pure geometry library (WKT literals, `geof::*` as plain Rust, `GeoIndex`) with no engine in the dependency graph — `geof_registry` / the registry then do not exist. The crate-level README front page (whose quickstart is a runnable doctest driving `geof_registry`) is attached only when `engine` is on (sq-wo5jw); the engine-less build gets a short pure-geometry crate doc, and the gating `sparq-geo pure-geometry (--no-default-features)` feature-matrix lane keeps that state (build/tests/doctests/clippy) green.
- **`reproject` feature (OFF by default)** adds the `reproject` module (pure-Rust `proj4rs`). It ships only a **small curated EPSG table**: projected 27700 (British National Grid), 3857 (Web Mercator), 2154 (Lambert-93), 25832/25833 (ETRS89 UTM 32/33N), and WGS84 UTM zones 326xx/327xx; plus geographic 4258 (ETRS89), 4269 (NAD83), 4283 (GDA94), 4171 (RGF93), 4490 (CGCS2000) — degree-valued, written LAT/LONG per the EPSG registry (`geographic_axis_order`), axis-normalised + datum-shifted into CRS84 by `to_crs84` (datums treated as WGS84-coincident, metre-level; sq-ove). NAD27 (4267) is deliberately unsupported (needs NADCON grids). The additional opt-in **`epsg_full` feature** (implies `reproject`; sq-248) embeds the full EPSG registry — the CC0 `crs-definitions` crate's proj4 strings, ~6k codes — as a FALLBACK behind the curated table (curated entries always win). The fallback stays honest: registry definitions needing shift grids we don't ship (`+datum=NAD27`, real `+nadgrids=` files) or carrying NO datum info on a non-WGS84-coincident ellipsoid are still refused, and registry `+proj=longlat` codes are assumed EPSG lat/long axis order. A PROJECTED registry entry is accepted only when its WKT's first two `AXIS[…,DIRECTION]` nodes match the directions its own PROJ.4 `+axis=` parameter declares to proj4rs (absent = easting/northing) — so easting/northing entries and the 29 explicitly axis-tagged ones (the southern-African Gauss conformal "Lo" grids, `+axis=wsu`; EPSG:8352, `+axis=swu`) are evaluated verbatim, while a no-AXIS export (Poland CS92, the ETRS89 "(N-E)" UTM variants, the Gauss-Krüger families), a transposed `NORTH,EAST` pair, or a WKT/PROJ.4 disagreement stays refused rather than silently transposed (issue #3752). Any other code is `GeoError::Unsupported`.
- **`topology_index` feature (OFF by default)** adds `GeoIndex::within_region_literals` and `contains_region_literals`: the index window-scans a geographic constant region's AABB, prepares that constant once, then DE-9IM-refines every candidate. These methods return the exact, deduplicated literal set for `sfWithin(literal, region)` / `sfContains(region, literal)`; empty and non-geographic constants return an empty set. The feature ALSO lights the engine's exact-candidate seam (a weak `sparq-engine?/spatial-exact-pushdown` feature edge): `GeoIndexProvider::candidates_exact` certifies this exact set to the engine, which then SKIPS the residual DE-9IM FILTER for certified rows — see the pushdown bullet below. [FABLE-5] sq-lk3aw.4
- **In SPARQL, every `geof:` failure is a per-row EXPRESSION error, not a hard query error**: a wrong datatype (must be a `geo:wktLiteral` **or** `geo:gmlLiteral`), unparseable WKT/GML, CRS mismatch, unknown unit IRI, or an unsupported operand combo drops the row in a `FILTER` / leaves the variable unbound in a `BIND` — the query still succeeds. But an **unregistered** `geof:` IRI (e.g. `geof:gmlToWkt`, which is not implemented) is the engine's usual hard "unsupported SPARQL function" error.
- **Relations are PLANAR DE-9IM** (via `geo`'s `Relate`) in coordinate/degree space, not geodesic. The hand-curated `tests/ogc_compliance_ratchet.rs` ratchet pins the exact DE-9IM truth value of every `sf*`/`eh*`/`rcc8*` relation over point/line/polygon/`MULTI*` operands in both orders (floor rises only with genuinely-passing assertions). `geof:distance` with metric units (`metre`/`kilometre`/`mile`) requires a geographic CRS (CRS84/EPSG:4326) and is **exact haversine for point↔point and point↔extended geometry** (spherical closest point via `HaversineClosestPoint`). **Extended↔extended geometry** uses **vertex-HaversineClosestPoint iteration** (sq-lk3aw.3): for each vertex of each operand the haversine distance to the nearest point on the other geometry is computed; the minimum is returned. This resolves the prior equirectangular projection distortion and is exact when the closest pair involves at least one vertex (the common case). Remaining approximation: interior-of-segment↔interior-of-segment closest pairs, bounded by vertex arc spacing — uncommon for typical GeoSPARQL geometries; a continent-spanning differential test (`tests/distance_differential.rs`) pins this to ≤ 1 % vs a Geodesic (WGS84 Karney) oracle. `geof:buffer` in metric units still uses a local equirectangular frame about the geometry's mean latitude. `Unit::Degree`/`Radian` measure euclidean coordinate-space distance.
- **Metric measurements are LOCAL-PLANAR.** `geof:metricArea`, `metricLength`, and `metricPerimeter` require CRS84/EPSG:4326 and use the same bounding-box-centred equirectangular metre frame as metric `buffer`; distortion grows with geographic extent and towards the poles. `metricArea` accepts polygonal inputs (holes subtracted; degenerate rings return zero), while `metricLength` / `metricPerimeter` accept curves and polygonal boundaries; dimensionally undefined inputs return `GeoError::Unsupported`. This strict unsupported dispatch intentionally follows the crate's honest-error policy, including `metricArea(POINT)`, rather than manufacturing a numeric result. `geof:centroid` returns a point in the input CRS; geographic inputs use that same frame and other CRSs use their coordinate space. [GPT-5.6] sq-lsp7k.18
- **Simplification uses coordinate-space Douglas–Peucker.** `geof:simplify(g, tolerance)` preserves the input CRS and retains only input vertices. Non-positive tolerances, points, multipoints, and two-vertex lines are unchanged. Line strings and polygon rings use `geo`'s Ramer–Douglas–Peucker implementation; it is not topology-preserving, so a simplified polygon can be invalid. Rectangles, triangles, geometry collections, and non-finite positive tolerances return `GeoError::Unsupported`. [GPT-5.6] sq-lsp7k.23
- **Set ops cover the point/line/polygon matrix.** Polygon×polygon `intersection`/`union`/`difference`/`symDifference` are exact (`geo` `BooleanOps`); the point and line cases are handled where well-defined — including **1-D set subtraction** (`line−line`, `line−polygon` and their `symDifference`), done via `i_overlay`'s string-line clip plus a linear-referencing collinear-overlap subtraction (point-set semantics: crossing points are measure-zero, so `line−line` drops only collinear overlaps; `line−polygon` keeps the line strictly outside the polygon closure; `polygon−line/point` is unchanged). The only remaining `GeoError::Unsupported` set-op cases are operands the dimension dispatch can't classify (e.g. a heterogeneous `GEOMETRYCOLLECTION`). Full operand matrix in the "Set-operation operand matrix" section below.
- **`GeoIndex` only indexes geographic-CRS, non-empty geometries** reached via `geo:asWKT` or `geo:asGML` (default graph + named graphs), with the indexed *entity* resolved through `geo:hasGeometry` / `geo:hasDefaultGeometry` when present (else the serialization subject itself). Other CRSs, empties, non-geometry-typed objects, and unparseable WKT/GML are counted in `index.skipped()` and excluded — check it to detect mis-typed data. `intersects` also requires a geographic-CRS argument.
- **Spatial FILTER pushdown (`SpatialProvider` seam) is a candidate SUPERSET, never a filter.** Wrap a `GeoIndex` as `GeoIndexProvider::new(index)` and install it with `sparq_engine::with_spatial_index(Arc<dyn SpatialProvider>, || …)` (alongside `with_functions(&geof_registry(), …)`): the engine recognises a pushable FILTER — `geof:sfWithin`/`sfIntersects(?g, box)` and `geof:distance(?g, pt, unit) OP r` with `OP ∈ {<,<=}` (metric units only; `>`/`>=` is an unbounded exterior and stays post-hoc) — asks the provider for the candidate literals over the geometry variable, restricts rows to them, and still runs the EXACT `geof:` FILTER. Results are byte-identical to running without the index; only fewer geometries are exact-checked. **Answer-safety:** a binding the index has NO opinion on (bound via a non-`geo:asWKT` predicate, a non-geographic CRS, a different graph, …) is NEVER dropped — it is kept for the exact FILTER. The keep decision is made at the ID level: `SpatialProvider::indexed_ids(dict_ptr)` (backed by `GeoIndex::indexed_ids_for`) returns the indexed-literal id-set as an `Arc<FxHashSet<Id>>` ONLY when `dict_ptr` (`std::ptr::from_ref(&graph.dict) as usize`) matches the dict the index was built over — a pure integer-lookup fast path that avoids per-row `Term` materialisation; on any dict mismatch it returns `None` and the engine falls back to the per-row `is_indexed(&Term)` check (always correct). Rebuild or `apply_delta` the inner index on data change and wrap afresh. [OPUS-4.8] sq-7jt80
- **EXACT-candidate pushdown (`spatial-exact-pushdown` engine feature, OFF by default; driven by sparq-geo's `topology_index`) may SKIP the residual `geof:` FILTER — but ONLY for provider-CERTIFIED rows.** For the within-region orientations — `geof:sfWithin(?g, REGION)` and (newly recognised) `geof:sfContains(REGION, ?g)`, Simple Features converses — the engine first asks `SpatialProvider::candidates_exact(SpatialExactQuery::WithinRegion)`: a `Some(v)` CERTIFIES `v` is EXACTLY the indexed bindings satisfying the predicate (no false positives/negatives over the indexed universe — a strictly stronger contract than `candidates`' superset). Rows are restricted to `certified ∪ not-indexed`; the residual FILTER is skipped only when EVERY surviving row is certified (then it is provably an identity). **Soundness invariant: a NOT-indexed binding is never certified — if any survives, the residual FILTER still runs (identity on certified rows, sole judge of the rest).** Declining (`None`, the default) falls back to the superset path. Differentials: `sparq-engine/tests/spatial_exact_pushdown.rs` (mock, mutation-checked) + `sparq-geo/tests/exact_pushdown.rs` (end-to-end, zero residual DE-9IM checks on an all-indexed corpus). [FABLE-5] sq-lk3aw.4
- **`geo:gmlLiteral` rides the SAME pipeline as WKT.** GML literals use the OGC **GML Simple-Features geometry profile** (`parse_gml_literal`): `gml:Point`/`LineString`/`Polygon` (exterior + interior rings)/`MultiPoint`/`MultiCurve`/`MultiSurface`, both GML 3 (`gml:pos`/`posList`/`exterior`) and GML 2 (`gml:coordinates`/`outerBoundaryIs`) spellings, namespace-prefix-agnostic, with `srsName`→CRS (URN/`EPSG:` forms; EPSG:4326 axis swap) matching the WKT path. `GeoGeometry::to_gml_literal` emits the GML 3 forms for the same six geometry classes, including holes and aggregates, with the inverse EPSG:4326 swap. [GPT-5.6] sq-i3wb5. An **empty** lexical form (or a member-less aggregate such as `<gml:MultiPoint/>` / `<gml:MultiGeometry/>`) is the empty geometry (OGC R16), the GML twin of the empty `wktLiteral`. A `geof:` call may mix a GML and a WKT argument. **Beyond GML-SF (parsed additively into the same 2-D model):** `gml:Envelope` (`gml:lowerCorner`/`upperCorner` → the 5-point bbox `Polygon`); arc-segment `gml:Curve`/`gml:Surface` — `gml:LineStringSegment` (linear) plus `gml:Arc`/`gml:ArcString`/`gml:CircularArcByCenterPoint`, **densified** to a polyline at a fixed `5°`-per-chord step (`gml::ARC_STEP_DEG`); this is an APPROXIMATION (no circular-arc type in `geo_types`), so densified topology/length/area are accurate only to that resolution; and `srsDimension="3"` coordinates, whose Z ordinate is parsed then **projected out** (the model is 2-D). **Still deferred (clean `GeoError::Unsupported`):** tessellated `gml:Triangle`/`gml:TIN` patches and non-circular (elliptic/clothoid) arc interpolations.
- **No RIF/`geor:` query rewriting, no `.hdt` write-out.** This is the GeoSPARQL *core*; see the crate's open beads (`bd list -l area:sparq-geo`) for the boundary.
- **Server wiring:** `sparq-server` exposes exactly this behind its opt-in `geo` cargo feature (`cargo build -p sparq-server --features geo`), which installs `geof_registry()` on every SPARQL endpoint via `with_functions`. With the feature off, the server and the wasm build carry no geometry code.
- **Reuse the registry:** `geof_registry()` is clone-cheap (`Arc`-shared fns) and `Send + Sync` — build it once (e.g. a `OnceLock`) and share across queries and threads.
- **Constant-geometry parse caching (sq-lkrgi):** the registry parses geometry arguments through a small bounded PER-THREAD cache keyed by (datatype, exact lexical form), so a constant geometry in a `FILTER`/`BIND` — e.g. Geographica's ~100 KB polygon constants — is parsed **once per thread, not once per row**. Sound because a lexical form parses to the same geometry deterministically (no graph state feeds the parse; no invalidation needed); WKT and GML are keyed separately; parse FAILURES are never cached (each erroneous row re-errors, preserving per-row expression-error semantics). Bounded by entry count and total key bytes; the MRU entry is never evicted, so even a single over-budget constant stays resident. [FABLE-5]
- **Prepared-relate caching (sq-hq8t5):** the DE-9IM relation families (`sf*`/`eh*`/`rcc8*`/`relate`) additionally cache a **prepared** form of each REUSED operand (`geo::PreparedGeometry`'s indexed topology graph — edge R\*-tree + self-nodes) in the same per-thread cache entry, the structure Jena's prepared-`GeometryWrapper` cache reuses. Built **lazily on the entry's first warm relate lookup** — a cache hit is the proof of reuse; single-use per-row geometries never pay preparation — and evicted with its entry under the same bounds. Results are **byte-identical** to unprepared relates (a prepared operand only substitutes a precomputed, equal `GeometryGraph`); differential tests pin registry-vs-plain equality across all 24 relations, `geof:relate` patterns, every prepared/unprepared dispatch arm, and the CRS-mismatch/malformed-pattern error strings. Honest boundary: this removes the constant side's per-row graph rebuild only — the STREAMING side's graph is still built per row (inherent: each row geometry is new), and no Geographica re-measurement has been recorded yet (the sq-lkrgi probe + Geographica harvest are the follow-up). [FABLE-5]

## Set-operation operand matrix

The four point-set operations dispatch on operand dimension:

| operands | `intersection` | `union` | `difference` (a − b) | `symDifference` |
|---|---|---|---|---|
| polygon × polygon | ✅ overlay | ✅ overlay | ✅ overlay | ✅ overlay |
| point × point | ✅ shared points | ✅ MULTIPOINT | ✅ set subtraction | ✅ set sym-diff |
| point × line/polygon | ✅ points on the other | ✅ GEOMETRYCOLLECTION | ✅ points not on the other | ✅ (point−other) ∪ other |
| line × line | ✅ crossings + collinear overlaps | ✅ MULTILINESTRING | ✅ collinear-overlap subtraction | ✅ (a−b) ∪ (b−a) |
| line × polygon | ✅ line clipped to the polygon | ✅ GEOMETRYCOLLECTION | ✅ line outside the polygon | ✅ (line outside) ∪ polygon |
| polygon × line/point | (symmetric) | (symmetric) | ✅ polygon unchanged (measure-zero) | ✅ (symmetric) |

Results are the lowest-dimension geometry capturing the answer and serialise back to `geo:wktLiteral`; line/line and line/polygon **unions** are plain set-unions (not noded/dissolved). **Roll-your-own + upstream:** `geo 0.33`'s `BooleanOps` overlays only polygons, so the line cases are done locally (`i_overlay`'s `FloatClip::clip_by` + an in-crate linear-referencing subtraction); a `geo`-side `LineString` difference is queued upstream for [georust/geo](https://github.com/georust/geo) (bead `sq-fxv3`). GML's parser is likewise a roll-your-own profile (no maintained pure-Rust GML *geometry* crate exists) queued for [georust](https://github.com/georust) (bead `sq-zy0`).

## Benchmarking the geo surface <!-- [FABLE-5] sq-hmd7l.29 -->

- **Per-commit HARD gate:** `bench/geo/run.sh` (fixed 100k-point CRS84 corpus; result-set-size +
  compliance counts asserted vs `bench/geo/expected.tsv` — COUNTS-NOT-COORDINATES).
- **Same-box competitor comparison:** `scripts/bench/geo-same-box.sh` (vs jena-fuseki-geosparql;
  opt-in `GEO_GEOGRAPHICA=1` adds the Geographica real-world LGD/GeoNames family — pinned recipe
  `bench/geo/geographica.sh`, pinned queries `bench/geo/queries-geographica/`). No timing is
  trusted without result-set-size agreement per query; see `bench/geo/README.md`.
- **Ad-hoc query timing:** `bench_geo query <corpus.nt> <query.rq> [iters]` (the crate example,
  default `engine` feature) — in-process `query_with_functions` + `geof_registry()`, emitting
  `name\tcount\tus`.
- **Cross-engine CONFORMANCE (not timing):** `bench/geo/gsb.sh` + the `gsb_compliance` example
  run sparq through the published 206-query GeoSPARQL Compliance Benchmark (ISPRS IJGI
  10(7):487, 2021) with the benchmark's own requirements-weighted scoring, so sparq sits on the same axis
  as the published Fuseki/GraphDB/Virtuoso percentages. Gather-only (the artifact is GPL-2.0
  and is never vendored). The recorded score, the configuration matrix behind it, and the
  per-requirement gaps — the largest by far being the query-rewrite extension, which currently
  *lowers* the score — are in `research/gap-conformance-cross-engine-2026-07.md` §3.7.
  <!-- [SONNET-4.6] sq-ql2iy -->

## See also

- `serve` / `sparql-query` sibling skills for running the engine and writing SPARQL.
- `hdt-format` and `fused-decompress-parse` for getting RDF into the `Graph` that `GeoIndex::build` and `geof_registry()` operate on.
