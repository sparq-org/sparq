<!-- [OPUS-4.8] sq-inzv: README brought to template. -->
# sparq-geo

<p>
  <a href="https://crates.io/crates/sparq-geo"><img src="https://img.shields.io/crates/v/sparq-geo.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-geo"><img src="https://docs.rs/sparq-geo/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

Opt-in GeoSPARQL 1.0/1.1 core for [sparq](https://github.com/sparq-org/sparq):
geometry-literal parsing and serialisation, `geof:` functions, and an R-tree `GeoIndex`.

A **separate crate** by design — no other sparq crate (nor the wasm build) depends on
it, so spatial support is engaged only by adding `sparq-geo`. Geometry wraps the
pure-Rust [`wkt`](https://crates.io/crates/wkt) / [`geo`](https://crates.io/crates/geo)
stack; the index wraps [`rstar`](https://crates.io/crates/rstar). GML has no maintained
pure-Rust geometry parser, so this crate ships a focused GML-SF parser
([`src/gml.rs`](src/gml.rs), on `quick-xml`), with an upstream `gml-geometry` proposal
prepared for [georust](https://github.com/georust) (bead `sq-zy0`).

## 🚀 Quickstart

`geof_registry()` (default-on `engine` feature) packages every implemented `geof:`
function as a [`sparq_engine::FunctionRegistry`](../../docs/extension-functions.md), so
they run inside real SPARQL `FILTER`/`BIND`:

```rust
# fn main() -> Result<(), String> {
use sparq_core::Graph;
use sparq_engine::query_with_functions;
use sparq_geo::geof_registry;

# let ttl = "<http://ex/london> <http://ex/loc> \"POINT(-0.1276 51.5074)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .\n<http://ex/paris> <http://ex/loc> \"POINT(2.3496 48.8530)\"^^<http://www.opengis.net/ont/geosparql#wktLiteral> .";
let g = Graph::load_str(ttl, "turtle")?;
let r = query_with_functions(&g, r#"
    PREFIX geof: <http://www.opengis.net/def/function/geosparql/>
    PREFIX uom:  <http://www.opengis.net/def/uom/OGC/1.0/>
    SELECT ?city WHERE {
      <http://ex/london> <http://ex/loc> ?here .
      ?city <http://ex/loc> ?there .
      FILTER(geof:distance(?here, ?there, uom:kilometre) < 400)
    }"#, &geof_registry())?;
# let _ = r;
# Ok(()) }
```

The `geof::*` / `geof::lex::*` plain-Rust API and the R-tree `GeoIndex`
(`within_distance` / `nearest` / `intersects_wkt`) are also available directly.

## ✨ Features

- **OGC conformance scoreboard** — 40 `geof:` IRIs cover topology, measurements, centroid, simplification, geometry/set operations, and `getSRID`. A self-written executable probe
  per OGC requirement ([`tests/ogc_geosparql_requirements.rs`](tests/ogc_geosparql_requirements.rs))
  scores **30 / 30** of the standard's R1–R30 taxonomy — OGC never shipped an executable
  ETS, and the GPL academic compliance benchmark cannot be vendored into this MIT tree,
  so this is sparq's own probe, not a TEAM-Engine run. A sibling hand-curated DE-9IM
  topology ratchet ([`tests/ogc_compliance_ratchet.rs`](tests/ogc_compliance_ratchet.rs))
  pins the exact truth value of every `sf*`/`eh*`/`rcc8*` relation over point/line/
  polygon/MULTI\* operands in both orders; its floor only rises with genuinely-passing,
  hand-derived assertions.
- **WKT + GML, two serializations** — `geo:wktLiteral` and the GML Simple-Features
  profile of `geo:gmlLiteral` parse to the same `geo_types` + CRS and interoperate in
  one `geof:` call. [GPT-5.6] `GeoGeometry::to_gml_literal` emits the six GML 3
  Simple Features forms with CRS-preserving axis order. Beyond GML-SF, non-SF forms are
  parsed additively into the same 2-D model: `gml:Envelope` (-> bbox `Polygon`),
  arc-segment `gml:Curve` / `gml:Surface` (`gml:Arc` / `gml:ArcString` /
  `gml:CircularArcByCenterPoint`), **densified** to a polyline at a fixed `5°`-per-chord
  step — an approximation, since `geo_types` has no circular-arc type — and 3-D
  (`srsDimension="3"`) coordinates, whose Z ordinate is parsed then projected out (the
  model is 2-D). Tessellated patches (`gml:Triangle` / `gml:TIN`) stay a clean
  `GeoError::Unsupported` (`bd list -l area:sparq-geo`).
- **CRS handling** — CRS84 (default) / EPSG:4326 (axis-order normalised); other CRS IRIs
  carried verbatim (relations valid within one CRS); opt-in CRS84 reprojection (pure-Rust
  proj4rs): a curated EPSG table (`reproject`) or the full EPSG registry (`epsg_full`).
- **RDFS/OWL entailment + query rewrite** — GeoSPARQL ontology entailment runs through
  the GENERIC `sparq-reason` closure (no geo-specific reasoner). The OGC query-rewrite
  extension (`/conf/query-rewrite-extension`) — topology PROPERTY forms like
  `?f geo:sfWithin ?region` — is the **opt-in `geosparql_rewrite` feature** (OFF by
  default): a dedicated [`geosparql_rewrite`](src/rewrite.rs) entry point expands them to
  a default-geometry resolution + matching `geof:` FILTER. The standard `sparq_engine`
  entry points stay W3C-conformant and untouched (a `geo:sfWithin` triple matches only
  asserted triples there), so default SPARQL semantics never change. A conformance
  ratchet pins the measured property-form pass count (`ogc_query_rewrite_ratchet`).
- **R-tree `GeoIndex`** — packed-STR `rstar` build over all graphs, with antimeridian-safe windows,
  incremental `apply_delta`, and opt-in prepared exact region scans via `topology_index` — which also certifies exact constant-region `sfWithin`/`sfContains` sets to the engine (`candidates_exact`), letting it skip the residual DE-9IM FILTER for indexed rows (sq-lk3aw.4).
- **Constant-geometry parse + prepared-relate caching** — geometry arguments resolve
  through a small bounded per-thread cache keyed by the exact lexical form: a constant
  `FILTER` polygon is parsed once per thread, not once per row (sq-lkrgi), and the
  DE-9IM relations lazily add a `geo::PreparedGeometry` per REUSED operand (sq-hq8t5).
  Results are byte-identical (differential-tested); parse failures are never cached.

**Distance accuracy.** Metric units measure great-circle distance on the GRS80 mean
sphere; point↔point and point↔extended geometry are exact haversine (spherical
closest-point). Extended↔extended geometry uses **vertex-HaversineClosestPoint
iteration** (sq-lk3aw.3): for each vertex of each geometry the haversine distance to
the nearest point on the other geometry is computed, resolving the prior equirectangular
projection distortion. Remaining approximation: interior-of-segment↔interior-of-segment
pairs (bounded by vertex arc spacing; uncommon for typical GeoSPARQL geometries).
`uom:degree`/`radian` measure coordinate-space distance. `geof:buffer` and the metric
area/length/perimeter functions use one local equirectangular frame; undefined
dimensions are errors, and `geof:centroid` preserves the input CRS. `geof:simplify`
uses coordinate-space Douglas–Peucker, retains input vertices, and preserves CRS. [GPT-5.6] sq-lsp7k.18 / sq-lsp7k.23. Line/polygon
set-subtraction is rolled in-crate over `i_overlay`; a `LineString` difference is proposed
upstream to [georust/geo](https://github.com/georust/geo) (bead `sq-fxv3`).

## 📚 Learn more

- **How-to** — [`skills/geosparql/SKILL.md`](../../skills/geosparql/SKILL.md) (the full
  `geof:` IRI list, the set-operation operand matrix, and the index design).
- **API reference** — [docs.rs/sparq-geo](https://docs.rs/sparq-geo).
- **Spec** — GeoSPARQL 1.0/1.1; the implemented requirement subset is pinned by
  [`tests/ogc_geosparql_requirements.rs`](tests/ogc_geosparql_requirements.rs).
- **Benchmark** — `cargo run --release -p sparq-geo --example bench_geo` (no figures
  baked in here, per the repo's no-hard-coded-performance-numbers rule); its `query`
  subcommand replays pinned `.rq` files (the Geographica family — `bench/geo/README.md`).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
