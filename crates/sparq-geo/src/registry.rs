//! [`geof_registry`]: the `geof:` functions packaged as a sparq-engine
//! [`FunctionRegistry`] (SPARQL 17.6 extensible value testing), so they run
//! inside SPARQL `FILTER` / `BIND` / `SELECT` expressions:
//!
//! ```
//! use sparq_core::Graph;
//! use sparq_geo::geof_registry;
//!
//! let g = Graph::load_str(r#"
//!     @prefix geo: <http://www.opengis.net/ont/geosparql#> .
//!     <http://ex/london> <http://ex/loc> "POINT(-0.1278 51.5074)"^^geo:wktLiteral .
//!     <http://ex/paris>  <http://ex/loc> "POINT(2.3522 48.8566)"^^geo:wktLiteral .
//! "#, "turtle").unwrap();
//! let r = sparq_engine::query_with_functions(&g,
//!     "PREFIX geof: <http://www.opengis.net/def/function/geosparql/>
//!      PREFIX uom:  <http://www.opengis.net/def/uom/OGC/1.0/>
//!      SELECT ?a ?b WHERE {
//!        ?a <http://ex/loc> ?ga . ?b <http://ex/loc> ?gb .
//!        FILTER(STR(?a) < STR(?b) && geof:distance(?ga, ?gb, uom:kilometre) < 400)
//!      }", &geof_registry()).unwrap();
//! assert_eq!(r.len(), 1); // London–Paris ≈ 343.6 km
//! ```
//!
//! Argument/result conventions (each function has the same semantics as its
//! typed [`crate::geof`] implementation):
//!
//! * geometry arguments must be `geo:wktLiteral` literals (anything else — wrong
//!   datatype, an IRI, a plain string — is a SPARQL *expression* error: the row is
//!   filtered by a `FILTER`, left unbound by a `BIND`; never a hard query error);
//! * `geof:distance`'s third argument is a unit IRI ([`crate::geof::Unit`]),
//!   result `xsd:double`;
//! * `geof:metricArea`, `metricLength`, and `metricPerimeter` return
//!   `xsd:double` in square metres / metres;
//! * `geof:maxX`, `minX`, `maxY`, and `minY` return the corresponding envelope
//!   coordinate as `xsd:double`;
//! * `geof:isEmpty` returns `xsd:boolean`;
//! * the `geof:sf*` relations return `xsd:boolean`;
//! * `geof:envelope` / `geof:boundary` / `geof:centroid` /
//!   `geof:convexHull` / `geof:simplify` return `geo:wktLiteral`;
//! * every [`crate::GeoError`] (WKT parse failure, CRS mismatch, unknown unit, …)
//!   is the same expression error.
//!
//! # Parsed-geometry caching (sq-lkrgi)
//!
//! The registry evaluates through the TYPED [`crate::geof`] API over a small
//! per-thread cache of parsed geometries keyed by (datatype, full lexical form)
//! — see `geom_cache` — instead of the string-level `geof::lex` mirrors, which
//! re-parse every argument on every row. A CONSTANT geometry in a `FILTER`
//! (Geographica's large-constant selections pass a ~100 KB polygon literal per
//! row) is therefore parsed once per thread, not once per row. This is a pure
//! evaluation-strategy change: a lexical form parses to the same geometry every
//! time (no graph state is involved), so results are identical to fresh parses;
//! the differential unit tests below pin that. Parse FAILURES are not cached —
//! an erroneous literal re-errors per row, exactly as before.
//!
//! # Prepared-relate caching (sq-hq8t5)
//!
//! The DE-9IM relation families (`geof:sf*` / `geof:eh*` / `geof:rcc8*` /
//! `geof:relate`) additionally cache a PREPARED form of each REUSED operand —
//! [`geo::PreparedGeometry`]'s indexed topology graph — in the same
//! `geom_cache` entry, built lazily on the entry's first warm relate lookup.
//! With the parse cache alone, relating a stream of rows against a large
//! constant polygon still rebuilt the constant's edge index and self-nodes on
//! every row (the dominant remaining per-row term on that shape); with the
//! prepared side cached, that work happens once per thread. See
//! `de9im_matrix` (code span: a private item — a link would break the
//! public-module rustdoc) for why the prepared path is result-identical —
//! the differential tests below pin it across all 24 relations and
//! `geof:relate`.

use crate::geof::{self, Unit};
use crate::literal::GeoGeometry;
use crate::vocab::{GEOF_NS, GML_LITERAL, WKT_LITERAL};
use crate::GeoError;
use geo::relate::IntersectionMatrix;
use geo::{PreparedGeometry, Relate};
use geo_types::Geometry;
use oxrdf::{Literal, NamedNode, Term};
use sparq_engine::FunctionRegistry;
use std::rc::Rc;

/// An OWNING prepared form of a parsed geometry for repeated DE-9IM relates
/// (sq-hq8t5).
///
/// `geo`'s `Relate` rebuilds a noded topology graph (edge R*-tree +
/// self-intersection nodes) for BOTH operands on EVERY call; against a large
/// constant polygon that rebuild dominates the per-row cost (the Geographica
/// q09/q13/q16/q17 shape — Jena caches prepared geometries for exactly this
/// reason). [`geo::PreparedGeometry`] hoists that per-operand work out of the
/// relate: it is built ONCE here and its graph is reused by every subsequent
/// relate via `Relate::geometry_graph`. Owning (`Geometry<f64>`, not a
/// borrow) so it can live in the per-thread `geom_cache` alongside the parsed
/// geometry with no self-reference.
///
/// The CRS lives on the parsed [`GeoGeometry`] the cache entry pairs this
/// with ([`RelateArg`] always carries both), so compatibility checking is
/// unchanged.
struct PreparedGeoGeometry {
    prepared: PreparedGeometry<'static, Geometry<f64>, f64>,
}

impl PreparedGeoGeometry {
    /// Indexes `g` once (edge R*-tree build + self-noding) for repeated
    /// relates. Infallible: preparation is a pure restructuring of the
    /// geometry, deferring nothing to the later relates.
    fn new(g: &GeoGeometry) -> Self {
        Self { prepared: PreparedGeometry::from(g.geometry.clone()) }
    }
}

/// One relate operand: the parsed geometry plus, when it came from a WARM
/// cache entry, its prepared form (sq-hq8t5).
struct RelateArg {
    geom: Rc<GeoGeometry>,
    prepared: Option<Rc<PreparedGeoGeometry>>,
}

/// The DE-9IM intersection matrix of `a` vs `b`, reusing a prepared side
/// when available (sq-hq8t5).
///
/// # Why the prepared path is result-identical (the load-bearing property)
///
/// All four arms run the same `geo` `RelateOperation` over the same robust
/// predicates; a prepared operand only substitutes a PRECOMPUTED
/// `GeometryGraph` (edge index + self-nodes) for the one a plain relate
/// would build fresh from the identical geometry — `geo`'s own tests pin
/// graph equality (`swap_arg_index`) and matrix equality
/// (`prepared_with_unprepared`), and this crate's differential tests below
/// re-pin matrix equality across all 24 relation functions and `geof:relate`
/// patterns. CRS compatibility is checked FIRST via the same
/// `geof::ensure_compatible` rule as the unprepared functions, so error
/// behaviour is unchanged too.
fn de9im_matrix(a: &RelateArg, b: &RelateArg) -> Result<IntersectionMatrix, String> {
    geof::ensure_compatible(&a.geom, &b.geom).map_err(|e| e.to_string())?;
    Ok(match (&a.prepared, &b.prepared) {
        (Some(pa), Some(pb)) => pa.prepared.relate(&pb.prepared),
        (Some(pa), None) => pa.prepared.relate(&b.geom.geometry),
        (None, Some(pb)) => a.geom.geometry.relate(&pb.prepared),
        (None, None) => a.geom.geometry.relate(&b.geom.geometry),
    })
}

/// How a relation family decides `true`/`false` from the DE-9IM matrix.
/// Splitting the relation table on this (instead of the whole-relation
/// `fn(&GeoGeometry, &GeoGeometry)` mirrors it used before sq-hq8t5) lets
/// every family share [`de9im_matrix`]'s prepared-side reuse.
#[derive(Clone, Copy)]
enum MatrixPred {
    /// A simple-features predicate method of [`IntersectionMatrix`].
    Sf(fn(&IntersectionMatrix) -> bool),
    /// ANY-of a GeoSPARQL matrix-pattern disjunction (Egenhofer / RCC8) —
    /// the same `geof` pattern consts the plain functions evaluate.
    Any(&'static [&'static str]),
}

impl MatrixPred {
    fn eval(self, m: &IntersectionMatrix) -> bool {
        match self {
            MatrixPred::Sf(p) => p(m),
            MatrixPred::Any(patterns) => geof::matrix_matches_any(m, patterns),
        }
    }
}

/// `Err` unless exactly `n` arguments were passed.
fn arity(name: &str, args: &[Term], n: usize) -> Result<(), String> {
    if args.len() == n {
        Ok(())
    } else {
        Err(format!("geof:{name} expects {n} argument(s), got {}", args.len()))
    }
}

/// Bounded per-thread cache of parsed geometry literals (sq-lkrgi).
///
/// # Why serving a cached parse is SOUND (the load-bearing property)
///
/// The cached value is `parse_wkt_literal(lex)` / `parse_gml_literal(lex)` for
/// the EXACT lexical form `lex` — a deterministic pure function of the string
/// alone (no graph, dictionary, or session state feeds the parse), so an entry
/// can never go stale and no invalidation is needed. The two parsers are keyed
/// separately ([`LexKind`]) so a lexical form cached under one datatype can
/// never satisfy a lookup for the other, and parse FAILURES are never cached
/// (each erroneous row re-errors, preserving per-row expression-error
/// semantics). The cache is per-thread (`thread_local!`, matching the
/// `sparq-vectors` mask-cache idiom) so the registry closures stay
/// `Send + Sync` with no locking, and it is bounded both by entry count and by
/// total key bytes so a long-lived process cannot grow it without bound.
mod geom_cache {
    use super::{GeoError, GeoGeometry, PreparedGeoGeometry};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Which parser a cache entry belongs to. WKT and GML lexical spaces are
    /// keyed separately: the same string means different things to the two
    /// parsers, so a hit must never cross datatypes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum LexKind {
        /// `geo:wktLiteral` — `crate::literal::parse_wkt_literal`.
        Wkt,
        /// `geo:gmlLiteral` — `crate::gml::parse_gml_literal`.
        Gml,
    }

    /// One cached parse: the key (parser + full lexical form), the shared
    /// parsed geometry, and — once the entry has been RE-used by a relate —
    /// its prepared form (sq-hq8t5).
    struct CachedGeom {
        kind: LexKind,
        lex: String,
        geom: Rc<GeoGeometry>,
        /// Lazily-built prepared form for the DE-9IM relate families, shared
        /// by every subsequent relate against this entry. `None` until the
        /// entry's first WARM relate lookup ([`parse_prepared`] hit):
        /// preparing costs about one relate's worth of per-operand graph
        /// work, so preparing EAGERLY on insert would tax the per-row
        /// streaming side (each distinct row geometry is inserted, related
        /// once, and evicted) for zero reuse. A hit is the proof of reuse —
        /// the constant side of a Geographica-style filter is prepared on
        /// its second row and reused for every row after. Evicted together
        /// with the entry, so the byte bound below caps prepared structures
        /// too (their size is proportional to the same lexical form).
        prepared: Option<Rc<PreparedGeoGeometry>>,
    }

    /// Entry cap. The hot shape is ONE constant + a stream of per-row
    /// geometries; a handful of slots keeps every constant of a realistic
    /// expression resident while per-row strings churn through the tail.
    pub(super) const MAX_ENTRIES: usize = 16;
    /// Total-key-bytes cap (the parsed geometry's size is roughly proportional
    /// to its lexical form, so key bytes are the memory proxy). Bounds the
    /// per-thread footprint even when every entry is a large polygon.
    pub(super) const MAX_KEY_BYTES: usize = 1 << 20;

    thread_local! {
        /// MRU-ordered (front = most recently used) cache. A `Vec` scanned
        /// linearly: at ≤ `MAX_ENTRIES` entries a scan is a few length checks
        /// plus one confirming memcmp on the hit, far below hash-map noise —
        /// and orders of magnitude below re-parsing a large WKT body.
        static CACHE: RefCell<Vec<CachedGeom>> = const { RefCell::new(Vec::new()) };
    }

    #[cfg(test)]
    thread_local! {
        /// Test-only count of parse ATTEMPTS (misses), successful or not.
        /// Lets tests assert a constant really was parsed once, not per row.
        static PARSE_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        /// Test-only count of PREPARED builds (sq-hq8t5). Lets tests assert a
        /// reused constant is prepared exactly once — not never (the cache
        /// would be inert) and not per row (the pathology it removes).
        static PREPARE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// The parsed geometry for `lex` under `kind`: a cache hit when this exact
    /// (kind, lexical form) was parsed on this thread recently, a fresh parse
    /// otherwise. Errors pass through uncached.
    pub(super) fn parse(kind: LexKind, lex: &str) -> Result<Rc<GeoGeometry>, GeoError> {
        Ok(lookup(kind, lex, false)?.0)
    }

    /// [`parse`] for a DE-9IM RELATE operand (sq-hq8t5): additionally returns
    /// the entry's prepared form, building it on the entry's first WARM relate
    /// lookup. A hit is the proof of reuse that pays for preparation; a miss
    /// (e.g. every distinct per-row geometry of a scan) never builds one, so
    /// the streaming side of a filter costs exactly what it did before.
    pub(super) fn parse_prepared(
        kind: LexKind,
        lex: &str,
    ) -> Result<(Rc<GeoGeometry>, Option<Rc<PreparedGeoGeometry>>), GeoError> {
        lookup(kind, lex, true)
    }

    /// Shared lookup: MRU-move on a hit (building the prepared form first if
    /// `prepare_on_hit` and the entry has none), parse-and-insert on a miss.
    fn lookup(
        kind: LexKind,
        lex: &str,
        prepare_on_hit: bool,
    ) -> Result<(Rc<GeoGeometry>, Option<Rc<PreparedGeoGeometry>>), GeoError> {
        // Lookup phase (borrow released before any parsing runs; the prepared
        // build is pure CPU over the entry's own geometry — no re-entry).
        let hit = CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            cache.iter().position(|e| e.kind == kind && e.lex == lex).map(|i| {
                if i > 0 {
                    let entry = cache.remove(i);
                    cache.insert(0, entry);
                }
                let entry = &mut cache[0];
                if prepare_on_hit && entry.prepared.is_none() {
                    #[cfg(test)]
                    PREPARE_COUNT.with(|c| c.set(c.get() + 1));
                    entry.prepared = Some(Rc::new(PreparedGeoGeometry::new(&entry.geom)));
                }
                (entry.geom.clone(), entry.prepared.clone())
            })
        });
        if let Some(hit) = hit {
            return Ok(hit);
        }
        #[cfg(test)]
        PARSE_ATTEMPTS.with(|c| c.set(c.get() + 1));
        let geom = Rc::new(match kind {
            LexKind::Wkt => crate::literal::parse_wkt_literal(lex)?,
            LexKind::Gml => crate::gml::parse_gml_literal(lex)?,
        });
        CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            cache.insert(
                0,
                CachedGeom { kind, lex: lex.to_string(), geom: geom.clone(), prepared: None },
            );
            // Evict from the LRU tail past either cap — but never the
            // just-inserted MRU entry, so even a single over-budget constant
            // stays cached (evicting it would re-parse it every row, the exact
            // pathology this cache removes).
            while cache.len() > 1
                && (cache.len() > MAX_ENTRIES
                    || cache.iter().map(|e| e.lex.len()).sum::<usize>() > MAX_KEY_BYTES)
            {
                cache.pop();
            }
        });
        Ok((geom, None))
    }

    /// Test-only: drop all entries and zero the counters so a test starts
    /// from a known-cold cache.
    #[cfg(test)]
    pub(super) fn reset() {
        CACHE.with(|c| c.borrow_mut().clear());
        PARSE_ATTEMPTS.with(|c| c.set(0));
        PREPARE_COUNT.with(|c| c.set(0));
    }

    /// Test-only: number of live entries.
    #[cfg(test)]
    pub(super) fn len() -> usize {
        CACHE.with(|c| c.borrow().len())
    }

    /// Test-only: parse attempts since the last [`reset`].
    #[cfg(test)]
    pub(super) fn parse_attempts() -> usize {
        PARSE_ATTEMPTS.with(|c| c.get())
    }

    /// Test-only: prepared-geometry builds since the last [`reset`] (sq-hq8t5).
    #[cfg(test)]
    pub(super) fn prepare_count() -> usize {
        PREPARE_COUNT.with(|c| c.get())
    }
}

/// The PARSED geometry of argument `i`, through the per-thread `geom_cache`
/// (so a constant geometry repeated across rows is parsed once per thread,
/// not once per row — sq-lkrgi). Both GeoSPARQL serializations are accepted
/// (§8.5): a `geo:wktLiteral` or a `geo:gmlLiteral`, each keyed under its own
/// parser. Any other term is a per-row expression error.
fn geom_arg(name: &str, args: &[Term], i: usize) -> Result<Rc<GeoGeometry>, String> {
    match &args[i] {
        Term::Literal(l) if l.datatype().as_str() == WKT_LITERAL => {
            geom_cache::parse(geom_cache::LexKind::Wkt, l.value()).map_err(|e| e.to_string())
        }
        Term::Literal(l) if l.datatype().as_str() == GML_LITERAL => {
            geom_cache::parse(geom_cache::LexKind::Gml, l.value()).map_err(|e| e.to_string())
        }
        other => Err(format!(
            "geof:{name}: argument {} must be a geo:wktLiteral or geo:gmlLiteral literal, got {other}",
            i + 1
        )),
    }
}

/// [`geom_arg`] for a DE-9IM RELATE operand (sq-hq8t5): same accepted terms
/// and error wording, but resolved through `geom_cache::parse_prepared` so a
/// REUSED operand (a constant geometry filtered against every row) also
/// carries its cached prepared form for [`de9im_matrix`].
fn relate_arg(name: &str, args: &[Term], i: usize) -> Result<RelateArg, String> {
    let parsed = match &args[i] {
        Term::Literal(l) if l.datatype().as_str() == WKT_LITERAL => {
            geom_cache::parse_prepared(geom_cache::LexKind::Wkt, l.value())
        }
        Term::Literal(l) if l.datatype().as_str() == GML_LITERAL => {
            geom_cache::parse_prepared(geom_cache::LexKind::Gml, l.value())
        }
        other => {
            return Err(format!(
                "geof:{name}: argument {} must be a geo:wktLiteral or geo:gmlLiteral literal, got {other}",
                i + 1
            ))
        }
    };
    let (geom, prepared) = parsed.map_err(|e| e.to_string())?;
    Ok(RelateArg { geom, prepared })
}

/// A `geo:wktLiteral` term from a lexical form produced by [`crate::geof::lex`].
fn wkt_term(lex: String) -> Term {
    Term::Literal(Literal::new_typed_literal(lex, NamedNode::new_unchecked(WKT_LITERAL)))
}

/// The unit-of-measure IRI in argument `i` (must be a named node).
fn unit_iri<'a>(name: &str, args: &'a [Term], i: usize) -> Result<&'a str, String> {
    match &args[i] {
        Term::NamedNode(unit) => Ok(unit.as_str()),
        other => Err(format!(
            "geof:{name}: argument {} must be a unit-of-measure IRI, got {other}",
            i + 1
        )),
    }
}

/// The numeric value of argument `i` (any literal whose lexical form parses as f64 —
/// the engine hands numerics over as their typed literals).
fn num_arg(name: &str, args: &[Term], i: usize) -> Result<f64, String> {
    match &args[i] {
        Term::Literal(l) => l.value().parse::<f64>().map_err(|_| {
            format!("geof:{name}: argument {} must be numeric, got {l}", i + 1)
        }),
        other => Err(format!("geof:{name}: argument {} must be numeric, got {other}", i + 1)),
    }
}

/// A [`crate::geof`] unary geometry function (`envelope` / `boundary` / `centroid` /
/// `convex_hull`).
type GeomUnary = fn(&GeoGeometry) -> Result<GeoGeometry, GeoError>;
/// A [`crate::geof`] unary numeric measurement function.
type NumericUnary = fn(&GeoGeometry) -> Result<f64, GeoError>;
/// A [`crate::geof`] unary boolean geometry predicate.
type BooleanUnary = fn(&GeoGeometry) -> Result<bool, GeoError>;
/// A [`crate::geof`] binary set operation (`intersection` / `union` / …).
type GeomSetOp = fn(&GeoGeometry, &GeoGeometry) -> Result<GeoGeometry, GeoError>;

/// The `geof:` extension functions as a sparq-engine [`FunctionRegistry`] — pass it
/// to [`sparq_engine::query_with_functions`] (or scope any other entry point with
/// [`sparq_engine::with_functions`]) to evaluate GeoSPARQL functions inside SPARQL.
///
/// Registered IRIs (all under `http://www.opengis.net/def/function/geosparql/`):
///
/// * `distance(g1, g2, unitIri)` -> `xsd:double`;
/// * `metricArea(g)` -> square-metre `xsd:double`, and `metricLength(g)` /
///   `metricPerimeter(g)` -> metre `xsd:double`;
/// * `maxX(g)` / `minX(g)` / `maxY(g)` / `minY(g)` -> the corresponding
///   envelope coordinate as `xsd:double`;
/// * `isEmpty(g)` -> `xsd:boolean`;
/// * the relation families, all `(g1, g2)` -> `xsd:boolean`: simple features
///   `sfEquals sfDisjoint sfIntersects sfTouches sfCrosses sfWithin sfContains
///   sfOverlaps`, Egenhofer `ehEquals ehDisjoint ehMeet ehOverlap ehCovers
///   ehCoveredBy ehInside ehContains`, RCC8 `rcc8eq rcc8dc rcc8ec rcc8po
///   rcc8tppi rcc8tpp rcc8ntpp rcc8ntppi`, plus the generic
///   `relate(g1, g2, de9imPattern)`;
/// * `envelope` / `boundary` / `centroid` / `convexHull` `(g)` ->
///   `geo:wktLiteral`;
/// * `simplify(g, tolerance)` -> Douglas–Peucker-simplified `geo:wktLiteral`;
/// * `buffer(g, radius, unitIri)` -> `geo:wktLiteral` (a MULTIPOLYGON);
/// * `intersection` / `union` / `difference` / `symDifference` `(g1, g2)` ->
///   `geo:wktLiteral` (point-set ops: polygon overlay plus the well-defined
///   line/point cases — see `geof` for the supported matrix);
/// * `getSRID(g)` -> `xsd:anyURI` (the geometry's CRS IRI);
/// * with the OPT-IN `geof_accessors` feature (OFF by default — the default
///   registry is unchanged), the GeoSPARQL 1.1 non-topological accessors:
///   `dimension` / `coordinateDimension` / `spatialDimension` `(g)` ->
///   `xsd:integer`, `isSimple(g)` -> `xsd:boolean`, and `geometryType(g)` ->
///   the OGC `sf:` geometry-class IRI as `xsd:anyURI` — delegating to the
///   `metadata` module's pure fns; undefined cases (the empty geometry's
///   dimensions / sf: class, a `GEOMETRYCOLLECTION`'s sf: class) are honest
///   expression errors, never fabricated values.
///
/// Build it once and reuse it: the registry is cheaply cloneable and `Send + Sync`,
/// so one instance can serve every query on every thread for the process lifetime.
///
/// Geometry arguments are parsed through a small bounded PER-THREAD cache keyed
/// by the literal's exact lexical form (sq-lkrgi), so a constant geometry in a
/// `FILTER` — even a ~100 KB polygon — is parsed once per thread rather than
/// once per row. Results are identical to fresh parses (the parse is a pure
/// function of the lexical form); parse failures are never cached.
///
/// The DE-9IM relation families additionally reuse a cached PREPARED form
/// (indexed topology graph) of any reused operand (sq-hq8t5), so relating
/// per-row geometries against a constant does not rebuild the constant's
/// relate structures per row. Results are byte-identical to the unprepared
/// functions (see the module docs).
pub fn geof_registry() -> FunctionRegistry {
    let mut reg = FunctionRegistry::new();

    // geof:distance(?g1, ?g2, ?unitIri) -> xsd:double.
    reg.register(format!("{GEOF_NS}distance"), |args: &[Term]| {
        arity("distance", args, 3)?;
        let a = geom_arg("distance", args, 0)?;
        let b = geom_arg("distance", args, 1)?;
        let unit = Unit::from_iri(unit_iri("distance", args, 2)?).map_err(|e| e.to_string())?;
        let d = geof::distance(&a, &b, unit).map_err(|e| e.to_string())?;
        Ok(Term::Literal(Literal::from(d)))
    });

    // Unary numeric functions: geometry -> xsd:double.
    let measurements: [(&'static str, NumericUnary); 7] = [
        ("metricArea", geof::metric_area),
        ("metricLength", geof::metric_length),
        ("metricPerimeter", geof::metric_perimeter),
        ("maxX", geof::max_x),
        ("minX", geof::min_x),
        ("maxY", geof::max_y),
        ("minY", geof::min_y),
    ];
    for (name, f) in measurements {
        reg.register(format!("{GEOF_NS}{name}"), move |args: &[Term]| {
            arity(name, args, 1)?;
            let g = geom_arg(name, args, 0)?;
            Ok(Term::Literal(Literal::from(f(&g).map_err(|e| e.to_string())?)))
        });
    }

    // Unary geometry predicates: geometry -> xsd:boolean. sq-lc2io
    let predicates: [(&'static str, BooleanUnary); 1] = [("isEmpty", geof::is_empty)];
    for (name, f) in predicates {
        reg.register(format!("{GEOF_NS}{name}"), move |args: &[Term]| {
            arity(name, args, 1)?;
            let g = geom_arg(name, args, 0)?;
            Ok(Term::Literal(Literal::from(f(&g).map_err(|e| e.to_string())?)))
        });
    }

    // The relation families: geof:sf* / geof:eh* / geof:rcc8*(?g1, ?g2) -> xsd:boolean.
    // Registered as (matrix predicate) over the shared prepared-aware
    // `de9im_matrix` (sq-hq8t5); each predicate is the SAME decision the
    // matching `geof` function makes (its `IntersectionMatrix` method for
    // simple features, its `geof` pattern const for Egenhofer/RCC8 — the
    // differential tests below pin the correspondence per name).
    let relations: [(&'static str, MatrixPred); 24] = [
        // Simple features (GeoSPARQL Req 22-24).
        ("sfEquals", MatrixPred::Sf(IntersectionMatrix::is_equal_topo)),
        ("sfDisjoint", MatrixPred::Sf(IntersectionMatrix::is_disjoint)),
        ("sfIntersects", MatrixPred::Sf(IntersectionMatrix::is_intersects)),
        ("sfTouches", MatrixPred::Sf(IntersectionMatrix::is_touches)),
        ("sfCrosses", MatrixPred::Sf(IntersectionMatrix::is_crosses)),
        ("sfWithin", MatrixPred::Sf(IntersectionMatrix::is_within)),
        ("sfContains", MatrixPred::Sf(IntersectionMatrix::is_contains)),
        ("sfOverlaps", MatrixPred::Sf(IntersectionMatrix::is_overlaps)),
        // Egenhofer (Req 25).
        ("ehEquals", MatrixPred::Any(geof::EH_EQUALS_PATTERNS)),
        ("ehDisjoint", MatrixPred::Any(geof::EH_DISJOINT_PATTERNS)),
        ("ehMeet", MatrixPred::Any(geof::EH_MEET_PATTERNS)),
        ("ehOverlap", MatrixPred::Any(geof::EH_OVERLAP_PATTERNS)),
        ("ehCovers", MatrixPred::Any(geof::EH_COVERS_PATTERNS)),
        ("ehCoveredBy", MatrixPred::Any(geof::EH_COVERED_BY_PATTERNS)),
        ("ehInside", MatrixPred::Any(geof::EH_INSIDE_PATTERNS)),
        ("ehContains", MatrixPred::Any(geof::EH_CONTAINS_PATTERNS)),
        // RCC8 (Req 26).
        ("rcc8eq", MatrixPred::Any(geof::RCC8_EQ_PATTERNS)),
        ("rcc8dc", MatrixPred::Any(geof::RCC8_DC_PATTERNS)),
        ("rcc8ec", MatrixPred::Any(geof::RCC8_EC_PATTERNS)),
        ("rcc8po", MatrixPred::Any(geof::RCC8_PO_PATTERNS)),
        ("rcc8tppi", MatrixPred::Any(geof::RCC8_TPPI_PATTERNS)),
        ("rcc8tpp", MatrixPred::Any(geof::RCC8_TPP_PATTERNS)),
        ("rcc8ntpp", MatrixPred::Any(geof::RCC8_NTPP_PATTERNS)),
        ("rcc8ntppi", MatrixPred::Any(geof::RCC8_NTPPI_PATTERNS)),
    ];
    for (name, pred) in relations {
        reg.register(format!("{GEOF_NS}{name}"), move |args: &[Term]| {
            arity(name, args, 2)?;
            let a = relate_arg(name, args, 0)?;
            let b = relate_arg(name, args, 1)?;
            let v = pred.eval(&de9im_matrix(&a, &b)?);
            Ok(Term::Literal(Literal::from(v)))
        });
    }

    // geof:relate(?g1, ?g2, ?de9imPattern) -> xsd:boolean (generic DE-9IM test).
    reg.register(format!("{GEOF_NS}relate"), |args: &[Term]| {
        arity("relate", args, 3)?;
        let a = relate_arg("relate", args, 0)?;
        let b = relate_arg("relate", args, 1)?;
        let pattern = match &args[2] {
            Term::Literal(l) => l.value(),
            other => {
                return Err(format!(
                    "geof:relate: argument 3 must be a DE-9IM pattern string, got {other}"
                ))
            }
        };
        // The same error `geof::relate` produces for a malformed pattern.
        let v = de9im_matrix(&a, &b)?.matches(pattern).map_err(|e| {
            GeoError::Parse(format!("invalid DE-9IM pattern {pattern:?}: {e}")).to_string()
        })?;
        Ok(Term::Literal(Literal::from(v)))
    });

    // The unary geometry functions: geof:*(?g) -> geo:wktLiteral.
    let unary: [(&'static str, GeomUnary); 4] = [
        ("envelope", geof::envelope),
        ("boundary", geof::boundary),
        ("centroid", geof::centroid),
        ("convexHull", geof::convex_hull),
    ];
    for (name, f) in unary {
        reg.register(format!("{GEOF_NS}{name}"), move |args: &[Term]| {
            arity(name, args, 1)?;
            let g = geom_arg(name, args, 0)?;
            Ok(wkt_term(f(&g).map_err(|e| e.to_string())?.to_wkt_literal()))
        });
    }

    // geof:simplify(?g, ?tolerance) -> geo:wktLiteral. sq-lsp7k.23
    reg.register(format!("{GEOF_NS}simplify"), |args: &[Term]| {
        arity("simplify", args, 2)?;
        let g = geom_arg("simplify", args, 0)?;
        let tolerance = num_arg("simplify", args, 1)?;
        Ok(wkt_term(
            geof::simplify(&g, tolerance)
                .map_err(|e| e.to_string())?
                .to_wkt_literal(),
        ))
    });

    // The set operations: geof:*(?g1, ?g2) -> geo:wktLiteral (point-set ops over
    // polygon/line/point operands; see geof for the supported matrix).
    let set_ops: [(&'static str, GeomSetOp); 4] = [
        ("intersection", geof::intersection),
        ("union", geof::union),
        ("difference", geof::difference),
        ("symDifference", geof::sym_difference),
    ];
    for (name, f) in set_ops {
        reg.register(format!("{GEOF_NS}{name}"), move |args: &[Term]| {
            arity(name, args, 2)?;
            let a = geom_arg(name, args, 0)?;
            let b = geom_arg(name, args, 1)?;
            Ok(wkt_term(f(&a, &b).map_err(|e| e.to_string())?.to_wkt_literal()))
        });
    }

    // geof:buffer(?g, ?radius, ?unitIri) -> geo:wktLiteral (a MULTIPOLYGON).
    reg.register(format!("{GEOF_NS}buffer"), |args: &[Term]| {
        arity("buffer", args, 3)?;
        let g = geom_arg("buffer", args, 0)?;
        let radius = num_arg("buffer", args, 1)?;
        let unit = Unit::from_iri(unit_iri("buffer", args, 2)?).map_err(|e| e.to_string())?;
        Ok(wkt_term(geof::buffer(&g, radius, unit).map_err(|e| e.to_string())?.to_wkt_literal()))
    });

    // geof:getSRID(?g) -> xsd:anyURI (the geometry's CRS IRI).
    reg.register(format!("{GEOF_NS}getSRID"), |args: &[Term]| {
        arity("getSRID", args, 1)?;
        let iri = geom_arg("getSRID", args, 0)?.crs.iri().to_string();
        Ok(Term::Literal(Literal::new_typed_literal(
            iri,
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#anyURI"),
        )))
    });

    // GeoSPARQL 1.1 non-topological ACCESSOR functions, behind the opt-in
    // `geof_accessors` feature (OFF by default): the default registry
    // byte-set is unchanged. sq-lsp7k
    #[cfg(feature = "geof_accessors")]
    register_accessors(&mut reg);

    reg
}

/// Registers the GeoSPARQL 1.1 non-topological ACCESSOR query functions
/// (opt-in `geof_accessors` feature) — `geof:dimension` /
/// `geof:coordinateDimension` / `geof:spatialDimension` -> `xsd:integer`,
/// `geof:isSimple` -> `xsd:boolean`, and `geof:geometryType` -> the OGC
/// `sf:` geometry-class IRI as `xsd:anyURI` — each parsing its single
/// geometry argument via [`geom_arg`] (same accepted terms, same cache) and
/// delegating to the [`crate::metadata`] pure fns. A value the metadata
/// layer leaves UNDEFINED — the empty geometry's dimensions, the sf: class
/// of an empty or `GEOMETRYCOLLECTION` operand — is an honest
/// [`GeoError::Unsupported`]-mapped expression error, never a fabricated
/// integer/boolean/IRI. sq-lsp7k
#[cfg(feature = "geof_accessors")]
fn register_accessors(reg: &mut FunctionRegistry) {
    use crate::metadata;

    // The integer accessors: geometry -> xsd:integer, each the matching
    // metadata.rs pure fn (`None` = undefined for the empty geometry).
    type MetaInt = fn(&Geometry<f64>) -> Option<u8>;
    let ints: [(&'static str, MetaInt); 3] = [
        ("dimension", metadata::topological_dimension),
        ("coordinateDimension", metadata::coordinate_dimension),
        ("spatialDimension", metadata::spatial_dimension),
    ];
    for (name, f) in ints {
        reg.register(format!("{GEOF_NS}{name}"), move |args: &[Term]| {
            arity(name, args, 1)?;
            let g = geom_arg(name, args, 0)?;
            let v = f(&g.geometry).ok_or_else(|| {
                GeoError::Unsupported(format!("geof:{name} is undefined for the empty geometry"))
                    .to_string()
            })?;
            Ok(Term::Literal(Literal::from(i64::from(v))))
        });
    }

    // geof:isSimple(?g) -> xsd:boolean. Total: the metadata.rs pure fn
    // defines every case (an EMPTY geometry is simple, per OGC SFA — a
    // defined answer, not a fabricated one).
    reg.register(format!("{GEOF_NS}isSimple"), |args: &[Term]| {
        arity("isSimple", args, 1)?;
        let g = geom_arg("isSimple", args, 0)?;
        Ok(Term::Literal(Literal::from(metadata::is_simple(&g.geometry))))
    });

    // geof:geometryType(?g) -> xsd:anyURI (the sf: geometry-class IRI, the
    // getSRID result convention). Declines — an expression error — where the
    // class is undefined: an EMPTY literal (geo-types canonicalises empties,
    // so the authored class is unrecoverable) or a GEOMETRYCOLLECTION.
    reg.register(format!("{GEOF_NS}geometryType"), |args: &[Term]| {
        arity("geometryType", args, 1)?;
        let g = geom_arg("geometryType", args, 0)?;
        let iri = metadata::sf_geometry_type(&g.geometry).ok_or_else(|| {
            GeoError::Unsupported(
                "geof:geometryType has no defined sf: class for an empty geometry \
                 or a GEOMETRYCOLLECTION"
                    .to_string(),
            )
            .to_string()
        })?;
        Ok(Term::Literal(Literal::new_typed_literal(
            iri,
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#anyURI"),
        )))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wkt(lex: &str) -> Term {
        wkt_term(lex.to_string())
    }

    #[test]
    fn registry_contents() {
        let reg = geof_registry();
        // 45 in the DEFAULT feature state; +5 accessors under the opt-in
        // `geof_accessors` feature (sq-lsp7k). The runtime `cfg!` keeps this
        // exact-count pin sound in EITHER feature state (a feature-unified
        // workspace build included).
        let expected = if cfg!(feature = "geof_accessors") { 50 } else { 45 };
        assert_eq!(reg.len(), expected);
        // The accessors are registered IFF the feature is on — the default
        // registry byte-set is unchanged (the sq-lsp7k invariant).
        for name in
            ["dimension", "coordinateDimension", "spatialDimension", "isSimple", "geometryType"]
        {
            assert_eq!(
                reg.get(&format!("{GEOF_NS}{name}")).is_some(),
                cfg!(feature = "geof_accessors"),
                "geof:{name} must be registered iff `geof_accessors` is on"
            );
        }
        for name in [
            "distance", "relate", "getSRID", "buffer",
            "metricArea", "metricLength", "metricPerimeter", "centroid",
            "maxX", "minX", "maxY", "minY", "isEmpty",
            "sfEquals", "sfDisjoint", "sfIntersects", "sfTouches", "sfCrosses",
            "sfWithin", "sfContains", "sfOverlaps",
            "ehEquals", "ehDisjoint", "ehMeet", "ehOverlap", "ehCovers", "ehCoveredBy",
            "ehInside", "ehContains",
            "rcc8eq", "rcc8dc", "rcc8ec", "rcc8po", "rcc8tppi", "rcc8tpp", "rcc8ntpp",
            "rcc8ntppi",
            "envelope", "boundary", "convexHull", "simplify",
            "intersection", "union", "difference", "symDifference",
        ] {
            assert!(reg.get(&format!("{GEOF_NS}{name}")).is_some(), "missing geof:{name}");
        }
    }

    #[test]
    fn measurement_functions_term_level() {
        let reg = geof_registry();
        let square = wkt("POLYGON((0 0, 1 0, 1 1, 0 1, 0 0))");

        for name in ["metricArea", "metricLength", "metricPerimeter"] {
            let f = reg.get(&format!("{GEOF_NS}{name}")).unwrap();
            let Term::Literal(value) = f(std::slice::from_ref(&square)).unwrap() else {
                panic!("literal")
            };
            assert_eq!(value.datatype().as_str(), "http://www.w3.org/2001/XMLSchema#double");
            assert!(value.value().parse::<f64>().unwrap() > 0.0, "geof:{name} returned {value}");
            assert!(f(&[]).is_err(), "geof:{name} must enforce unary arity");
        }

        let centroid = reg.get(&format!("{GEOF_NS}centroid")).unwrap();
        let Term::Literal(value) = centroid(&[square]).unwrap() else { panic!("literal") };
        assert_eq!(value.datatype().as_str(), WKT_LITERAL);
        assert!(value.value().contains("0.5"), "got {value}");
    }

    #[test]
    fn bounding_coordinate_function_term_level() {
        let reg = geof_registry();
        let polygon = wkt("POLYGON((0 0, 4 0, 4 3, 0 3, 0 0))");
        let max_x = reg
            .get(&format!("{GEOF_NS}maxX"))
            .expect("registered geof:maxX");

        let Term::Literal(value) = max_x(std::slice::from_ref(&polygon)).unwrap() else {
            panic!("literal")
        };
        assert_eq!(value.datatype().as_str(), "http://www.w3.org/2001/XMLSchema#double");
        assert!((value.value().parse::<f64>().unwrap() - 4.0).abs() < 1e-12);
        assert!(max_x(&[]).is_err(), "geof:maxX must enforce unary arity");
    }

    #[test]
    fn is_empty_term_level() {
        let reg = geof_registry();
        let is_empty = reg
            .get(&format!("{GEOF_NS}isEmpty"))
            .expect("registered geof:isEmpty");

        for (wkt_lexical, expected) in [
            ("POINT(1 2)", false),
            ("POINT EMPTY", true),
            ("LINESTRING EMPTY", true),
            ("POLYGON((0 0,1 0,1 1,0 1,0 0))", false),
        ] {
            let Term::Literal(value) = is_empty(&[wkt(wkt_lexical)]).unwrap() else {
                panic!("literal")
            };
            assert_eq!(
                value.datatype().as_str(),
                "http://www.w3.org/2001/XMLSchema#boolean"
            );
            assert_eq!(value.value(), expected.to_string(), "WKT: {wkt_lexical}");
        }
        assert!(
            is_empty(&[]).is_err(),
            "geof:isEmpty must enforce unary arity"
        );
    }

    #[test]
    fn simplify_term_level() {
        let reg = geof_registry();
        let simplify = reg.get(&format!("{GEOF_NS}simplify")).unwrap();
        let tolerance = Term::Literal(Literal::from(0.2));
        let Term::Literal(value) =
            simplify(&[wkt("LINESTRING(0 0,1 0.1,2 0)"), tolerance]).unwrap()
        else {
            panic!("literal")
        };

        assert_eq!(value.datatype().as_str(), WKT_LITERAL);
        assert_eq!(value.value(), "LINESTRING(0 0,2 0)");
        assert!(simplify(&[wkt("POINT(0 0)")]).is_err());
        assert!(simplify(&[
            wkt("POINT(0 0)"),
            Term::Literal(Literal::new_simple_literal("not-a-number")),
        ])
        .is_err());
    }

    #[test]
    fn get_srid_term_level() {
        let reg = geof_registry();
        let f = reg.get(&format!("{GEOF_NS}getSRID")).unwrap();
        let Term::Literal(l) = f(&[wkt("POINT(1 2)")]).unwrap() else { panic!("literal") };
        assert_eq!(l.value(), crate::vocab::CRS84);
        assert_eq!(l.datatype().as_str(), "http://www.w3.org/2001/XMLSchema#anyURI");
        let Term::Literal(l) =
            f(&[wkt("<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(1 2)")]).unwrap()
        else {
            panic!("literal")
        };
        assert_eq!(l.value(), "http://www.opengis.net/def/crs/EPSG/0/27700");
    }

    #[test]
    fn relate_term_level() {
        let reg = geof_registry();
        let f = reg.get(&format!("{GEOF_NS}relate")).unwrap();
        let a = wkt("POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))");
        let b = wkt("POINT(1 1)");
        let pat = |p: &str| Term::Literal(Literal::new_simple_literal(p));
        // "contains" pattern.
        assert_eq!(
            f(&[a.clone(), b.clone(), pat("T*****FF*")]).unwrap().to_string(),
            "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"
        );
        // "disjoint" pattern is false for a contained point.
        assert_eq!(
            f(&[a.clone(), b.clone(), pat("FF*FF****")]).unwrap().to_string(),
            "\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>"
        );
        // Malformed pattern / non-string pattern are expression errors.
        assert!(f(&[a.clone(), b.clone(), pat("TTT")]).is_err());
        assert!(f(&[a.clone(), b.clone(), Term::NamedNode(NamedNode::new_unchecked("http://x/"))]).is_err());
    }

    #[test]
    fn set_operations_and_buffer_term_level() {
        let reg = geof_registry();
        let a = wkt("POLYGON((0 0, 2 0, 2 2, 0 2, 0 0))");
        let b = wkt("POLYGON((1 1, 3 1, 3 3, 1 3, 1 1))");
        let inter = reg.get(&format!("{GEOF_NS}intersection")).unwrap();
        let Term::Literal(l) = inter(&[a.clone(), b.clone()]).unwrap() else { panic!("literal") };
        assert_eq!(l.datatype().as_str(), WKT_LITERAL);
        assert!(l.value().contains("MULTIPOLYGON"), "got {}", l.value());
        // Line/point operands are now supported (sq-gn3): a point inside the
        // polygon survives; a point outside clips to an empty MULTIPOINT.
        let Term::Literal(inside) = inter(&[wkt("POINT(2 2)"), b.clone()]).unwrap() else {
            panic!("literal")
        };
        assert!(inside.value().contains('2'), "got {}", inside.value());
        let Term::Literal(outside) = inter(&[wkt("POINT(0 0)"), b.clone()]).unwrap() else {
            panic!("literal")
        };
        assert!(
            outside.value().contains("EMPTY") || outside.value().contains("()"),
            "got {}",
            outside.value()
        );
        // 1-D set subtraction is now supported (sq-fxv3): line − line removes
        // the collinear-overlapping span, leaving LINESTRING(0 0, 1 0).
        let diff = reg.get(&format!("{GEOF_NS}difference")).unwrap();
        let Term::Literal(d) =
            diff(&[wkt("LINESTRING(0 0, 2 0)"), wkt("LINESTRING(1 0, 3 0)")]).unwrap()
        else {
            panic!("literal")
        };
        assert_eq!(d.datatype().as_str(), WKT_LITERAL);
        assert!(d.value().contains("LINESTRING") && d.value().contains('1'), "got {}", d.value());
        // buffer: polygon out, radius must be numeric, unit must be known.
        let buffer = reg.get(&format!("{GEOF_NS}buffer")).unwrap();
        let metre = Term::NamedNode(NamedNode::new_unchecked(format!("{}metre", crate::vocab::UOM_NS)));
        let radius = Term::Literal(Literal::from(100.0));
        let Term::Literal(l) = buffer(&[wkt("POINT(0 51)"), radius.clone(), metre.clone()]).unwrap()
        else {
            panic!("literal")
        };
        assert!(l.value().contains("MULTIPOLYGON"), "got {}", l.value());
        assert!(buffer(&[wkt("POINT(0 51)"), wkt("POINT(0 0)"), metre]).is_err());
        assert!(buffer(&[wkt("POINT(0 51)"), radius, Term::NamedNode(NamedNode::new_unchecked("http://ex/furlong"))]).is_err());
    }

    #[test]
    fn distance_term_level() {
        let reg = geof_registry();
        let f = reg.get(&format!("{GEOF_NS}distance")).unwrap();
        let unit = Term::NamedNode(NamedNode::new_unchecked(format!("{}kilometre", crate::vocab::UOM_NS)));
        let out = f(&[wkt("POINT(-0.1278 51.5074)"), wkt("POINT(2.3522 48.8566)"), unit]).unwrap();
        let Term::Literal(l) = out else { panic!("expected a literal") };
        assert_eq!(l.datatype().as_str(), "http://www.w3.org/2001/XMLSchema#double");
        let km: f64 = l.value().parse().unwrap();
        assert!((km - 343.6).abs() < 1.0, "London–Paris ≈ 343.6 km, got {km}");
    }

    #[test]
    fn relation_and_unary_term_level() {
        let reg = geof_registry();
        let within = reg.get(&format!("{GEOF_NS}sfWithin")).unwrap();
        let poly = wkt("POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))");
        assert_eq!(
            within(&[wkt("POINT(1 1)"), poly.clone()]).unwrap().to_string(),
            "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"
        );
        assert_eq!(
            within(&[wkt("POINT(9 9)"), poly.clone()]).unwrap().to_string(),
            "\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>"
        );
        let envelope = reg.get(&format!("{GEOF_NS}envelope")).unwrap();
        let Term::Literal(l) = envelope(&[poly]).unwrap() else { panic!("expected a literal") };
        assert_eq!(l.datatype().as_str(), WKT_LITERAL);
        assert!(l.value().contains("POLYGON"), "got {}", l.value());
    }

    #[test]
    fn bad_arguments_are_errs() {
        let reg = geof_registry();
        let within = reg.get(&format!("{GEOF_NS}sfWithin")).unwrap();
        // Wrong arity.
        assert!(within(&[wkt("POINT(1 1)")]).is_err());
        // Not a wktLiteral (plain xsd:string).
        assert!(within(&[Term::Literal(Literal::new_simple_literal("POINT(1 1)")), wkt("POINT(1 1)")]).is_err());
        // Unparsable WKT.
        assert!(within(&[wkt("PONT(1 1)"), wkt("POINT(1 1)")]).is_err());
        // distance: unit must be an IRI, and a known one.
        let distance = reg.get(&format!("{GEOF_NS}distance")).unwrap();
        assert!(distance(&[wkt("POINT(0 0)"), wkt("POINT(1 1)"), wkt("POINT(0 0)")]).is_err());
        let bogus = Term::NamedNode(NamedNode::new_unchecked("http://ex/furlong"));
        assert!(distance(&[wkt("POINT(0 0)"), wkt("POINT(1 1)"), bogus]).is_err());
    }

    // ---- the parsed-geometry cache (sq-lkrgi) -----------------------------
    //
    // Each test runs on its own thread (the default test harness), so the
    // thread-local cache and attempt counter are naturally isolated; `reset()`
    // makes the cold start explicit.

    /// A syntactically valid ring POLYGON with `vertices` distinct vertices
    /// (radius-2 circle about the origin) — a scaled-down stand-in for the
    /// Geographica large-constant polygon.
    fn ring_polygon(vertices: usize) -> String {
        let mut s = String::from("POLYGON((");
        for i in 0..=vertices {
            let theta = (i % vertices) as f64 / vertices as f64 * std::f64::consts::TAU;
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("{} {}", 2.0 * theta.cos(), 2.0 * theta.sin()));
        }
        s.push_str("))");
        s
    }

    /// A syntactically valid LINESTRING whose lexical form exceeds `min_len` bytes.
    fn big_linestring(min_len: usize) -> String {
        let mut s = String::with_capacity(min_len + 32);
        s.push_str("LINESTRING(0 0");
        let mut i = 1usize;
        while s.len() < min_len {
            s.push_str(&format!(", {} {}", i, i % 7));
            i += 1;
        }
        s.push(')');
        s
    }

    #[test]
    fn constant_geometry_is_parsed_once_across_rows() {
        geom_cache::reset();
        let reg = geof_registry();
        let within = reg.get(&format!("{GEOF_NS}sfWithin")).unwrap();
        // One constant polygon FILTERed against a stream of distinct per-row
        // points — the Geographica q09/q13/q16/q17 shape.
        let constant = wkt(&ring_polygon(512));
        let rows = 50;
        for i in 0..rows {
            let p = wkt(&format!("POINT({} 0.0)", i as f64 * 0.001));
            assert_eq!(
                within(&[p, constant.clone()]).unwrap().to_string(),
                "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"
            );
        }
        // `rows` distinct per-row points + ONE parse of the constant. Without
        // the cache this is `2 * rows` (the constant re-parsed every row).
        assert_eq!(geom_cache::parse_attempts(), rows + 1);
    }

    #[test]
    fn cached_parse_equals_fresh_parse() {
        // The load-bearing equivalence: a cache HIT returns exactly what a
        // fresh parse returns (incl. CRS handling and EPSG:4326 axis swap).
        geom_cache::reset();
        for lex in [
            "POINT(-0.1278 51.5074)",
            "<http://www.opengis.net/def/crs/EPSG/0/4326> POINT(51.5074 -0.1278)",
            "<http://www.opengis.net/def/crs/EPSG/0/27700> LINESTRING(530000 180000, 531000 181000)",
            "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))",
        ] {
            let cold = geom_cache::parse(geom_cache::LexKind::Wkt, lex).unwrap();
            let warm = geom_cache::parse(geom_cache::LexKind::Wkt, lex).unwrap();
            let fresh = crate::literal::parse_wkt_literal(lex).unwrap();
            assert_eq!(*cold, fresh, "cold parse must equal a fresh parse for {}", lex);
            assert!(Rc::ptr_eq(&cold, &warm), "second lookup must be a cache hit for {}", lex);
            assert_eq!(*warm, fresh, "cached parse must equal a fresh parse for {}", lex);
        }
        // Same equivalence for the GML parser under its own key space.
        let gml = "<gml:Point><gml:pos>-83.38 33.95</gml:pos></gml:Point>";
        let cold = geom_cache::parse(geom_cache::LexKind::Gml, gml).unwrap();
        let warm = geom_cache::parse(geom_cache::LexKind::Gml, gml).unwrap();
        let fresh = crate::gml::parse_gml_literal(gml).unwrap();
        assert_eq!(*cold, fresh);
        assert!(Rc::ptr_eq(&cold, &warm));
    }

    #[test]
    fn wkt_and_gml_cache_keys_do_not_collide() {
        geom_cache::reset();
        // Cache a string under the WKT parser…
        let kept = geom_cache::parse(geom_cache::LexKind::Wkt, "POINT(1 2)").unwrap();
        // …then look the SAME string up under the GML kind: it must NOT serve
        // the WKT entry — it must attempt (and fail) a GML parse of non-XML.
        assert!(geom_cache::parse(geom_cache::LexKind::Gml, "POINT(1 2)").is_err());
        assert_eq!(geom_cache::parse_attempts(), 2);
        drop(kept);
    }

    #[test]
    fn cache_entry_count_is_bounded_and_mru_survives() {
        geom_cache::reset();
        for i in 0..(geom_cache::MAX_ENTRIES + 8) {
            geom_cache::parse(geom_cache::LexKind::Wkt, &format!("POINT({} 0)", i)).unwrap();
        }
        assert!(geom_cache::len() <= geom_cache::MAX_ENTRIES);
        // The most recently used entry is still resident (no new attempt)…
        let attempts = geom_cache::parse_attempts();
        let last = format!("POINT({} 0)", geom_cache::MAX_ENTRIES + 7);
        geom_cache::parse(geom_cache::LexKind::Wkt, &last).unwrap();
        assert_eq!(geom_cache::parse_attempts(), attempts);
        // …while the oldest was evicted (a lookup re-parses it).
        geom_cache::parse(geom_cache::LexKind::Wkt, "POINT(0 0)").unwrap();
        assert_eq!(geom_cache::parse_attempts(), attempts + 1);
    }

    #[test]
    fn byte_cap_evicts_tail_but_never_the_mru_entry() {
        geom_cache::reset();
        // A single constant LARGER than the whole byte budget still caches —
        // the MRU entry is never evicted (evicting it would re-parse it every
        // row, the exact pathology the cache removes).
        let big = big_linestring(geom_cache::MAX_KEY_BYTES + 1024);
        geom_cache::parse(geom_cache::LexKind::Wkt, &big).unwrap();
        assert_eq!(geom_cache::len(), 1);
        geom_cache::parse(geom_cache::LexKind::Wkt, &big).unwrap();
        assert_eq!(geom_cache::parse_attempts(), 1, "over-budget constant must still cache");
        // A subsequent small insert pushes the over-budget giant out of the tail…
        geom_cache::parse(geom_cache::LexKind::Wkt, "POINT(3 4)").unwrap();
        assert_eq!(geom_cache::len(), 1, "byte cap must evict the giant tail entry");
        // …and the small MRU entry is the survivor.
        geom_cache::parse(geom_cache::LexKind::Wkt, "POINT(3 4)").unwrap();
        assert_eq!(geom_cache::parse_attempts(), 2);
    }

    #[test]
    fn parse_failures_are_not_cached() {
        geom_cache::reset();
        assert!(geom_cache::parse(geom_cache::LexKind::Wkt, "PONT(1 1)").is_err());
        assert!(geom_cache::parse(geom_cache::LexKind::Wkt, "PONT(1 1)").is_err());
        // Each erroneous row re-attempts (and re-errors); nothing is stored.
        assert_eq!(geom_cache::parse_attempts(), 2);
        assert_eq!(geom_cache::len(), 0);
    }

    // ---- the prepared-relate cache (sq-hq8t5) -----------------------------

    /// Parses a WKT lexical form directly (no cache — the differential
    /// baseline the prepared path must match).
    fn parse(lex: &str) -> GeoGeometry {
        crate::literal::parse_wkt_literal(lex).unwrap()
    }

    /// The lexical `Term` form of an `xsd:boolean` result.
    fn bool_term(v: bool) -> String {
        format!("\"{v}\"^^<http://www.w3.org/2001/XMLSchema#boolean>")
    }

    /// A small corpus covering the DE-9IM decision space: containment,
    /// touching, overlap, disjoint, crossing, a hole, 0/1/2-dimensional and
    /// multi-part operands. Self-pairs and both orders are exercised by the
    /// differential tests' full pair loop.
    const RELATE_CORPUS: [&str; 9] = [
        "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))",
        "POLYGON((1 1, 3 1, 3 3, 1 3, 1 1))",
        "POLYGON((4 0, 8 0, 8 4, 4 4, 4 0))",
        "POLYGON((10 10, 12 10, 12 12, 10 12, 10 10))",
        "POLYGON((2 2, 6 2, 6 6, 2 6, 2 2))",
        "POLYGON((0 0, 8 0, 8 8, 0 8, 0 0), (2 2, 6 2, 6 6, 2 6, 2 2))",
        "LINESTRING(-1 -1, 5 5)",
        "POINT(1 1)",
        "MULTIPOINT((1 1), (9 9))",
    ];

    #[test]
    fn constant_relate_operand_is_prepared_once_across_rows() {
        geom_cache::reset();
        let reg = geof_registry();
        let within = reg.get(&format!("{GEOF_NS}sfWithin")).unwrap();
        // One constant polygon FILTERed against a stream of distinct per-row
        // points — the Geographica q09/q13/q16/q17 shape the bead targets.
        let constant = wkt(&ring_polygon(512));
        let rows = 50;
        for i in 0..rows {
            let p = wkt(&format!("POINT({} 0.0)", i as f64 * 0.001));
            assert_eq!(within(&[p, constant.clone()]).unwrap().to_string(), bool_term(true));
        }
        // The constant is prepared exactly ONCE (on its first warm row):
        // 0 would mean the prepared cache is inert; `rows - 1` (or anything
        // per-row) is the rebuild-per-row pathology this bead removes. The
        // distinct per-row points are never prepared (each is seen once).
        assert_eq!(geom_cache::prepare_count(), 1);
        // Parse-cache behaviour is unchanged by the prepared layer.
        assert_eq!(geom_cache::parse_attempts(), rows + 1);
    }

    #[test]
    fn single_use_relate_operands_are_never_prepared() {
        geom_cache::reset();
        let reg = geof_registry();
        let intersects = reg.get(&format!("{GEOF_NS}sfIntersects")).unwrap();
        // Every operand distinct (the pure streaming shape): preparation
        // would be pay-once-use-once overhead, so none may happen.
        for i in 0..20 {
            let a = wkt(&format!("POINT({i} 0)"));
            let b = wkt(&format!("POLYGON(({i} -1, {} -1, {} 1, {i} 1, {i} -1))", i + 2, i + 2));
            assert_eq!(intersects(&[a, b]).unwrap().to_string(), bool_term(true));
        }
        assert_eq!(geom_cache::prepare_count(), 0);
    }

    #[test]
    fn de9im_matrix_arms_all_equal_plain_relate() {
        use geo::Relate as _;
        // Every (prepared?, prepared?) dispatch arm must produce the exact
        // matrix of a plain relate — for every ordered corpus pair.
        for la in RELATE_CORPUS {
            for lb in RELATE_CORPUS {
                let (a, b) = (parse(la), parse(lb));
                let plain = format!("{:?}", a.geometry.relate(&b.geometry));
                let arg = |g: &GeoGeometry, prepared: bool| RelateArg {
                    geom: Rc::new(g.clone()),
                    prepared: prepared.then(|| Rc::new(PreparedGeoGeometry::new(g))),
                };
                for (pa, pb) in [(false, false), (true, false), (false, true), (true, true)] {
                    let m = de9im_matrix(&arg(&a, pa), &arg(&b, pb)).unwrap();
                    assert_eq!(
                        format!("{m:?}"),
                        plain,
                        "matrix mismatch for {la} vs {lb} (prepared: {pa}/{pb})"
                    );
                }
            }
        }
        // CRS compatibility errors identically regardless of preparation.
        let a = parse("<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(1 2)");
        let b = parse("POINT(1 2)");
        let expected = geof::sf_equals(&a, &b).unwrap_err().to_string();
        let pa = RelateArg {
            geom: Rc::new(a.clone()),
            prepared: Some(Rc::new(PreparedGeoGeometry::new(&a))),
        };
        let pb = RelateArg { geom: Rc::new(b), prepared: None };
        assert_eq!(de9im_matrix(&pa, &pb).unwrap_err(), expected);
    }

    #[test]
    fn prepared_relation_results_equal_unprepared_for_all_24_relations() {
        // The registry's (name -> matrix predicate) table must decide exactly
        // like the plain `geof` function of the same name — including via the
        // prepared path. A scrambled table entry (e.g. ehMeet <-> ehOverlap)
        // or a prepared-side divergence turns this red.
        type PlainRelation = fn(&GeoGeometry, &GeoGeometry) -> Result<bool, GeoError>;
        let plain: [(&str, PlainRelation); 24] = [
            ("sfEquals", geof::sf_equals),
            ("sfDisjoint", geof::sf_disjoint),
            ("sfIntersects", geof::sf_intersects),
            ("sfTouches", geof::sf_touches),
            ("sfCrosses", geof::sf_crosses),
            ("sfWithin", geof::sf_within),
            ("sfContains", geof::sf_contains),
            ("sfOverlaps", geof::sf_overlaps),
            ("ehEquals", geof::eh_equals),
            ("ehDisjoint", geof::eh_disjoint),
            ("ehMeet", geof::eh_meet),
            ("ehOverlap", geof::eh_overlap),
            ("ehCovers", geof::eh_covers),
            ("ehCoveredBy", geof::eh_covered_by),
            ("ehInside", geof::eh_inside),
            ("ehContains", geof::eh_contains),
            ("rcc8eq", geof::rcc8_eq),
            ("rcc8dc", geof::rcc8_dc),
            ("rcc8ec", geof::rcc8_ec),
            ("rcc8po", geof::rcc8_po),
            ("rcc8tppi", geof::rcc8_tppi),
            ("rcc8tpp", geof::rcc8_tpp),
            ("rcc8ntpp", geof::rcc8_ntpp),
            ("rcc8ntppi", geof::rcc8_ntppi),
        ];
        geom_cache::reset();
        let reg = geof_registry();
        for la in RELATE_CORPUS {
            for lb in RELATE_CORPUS {
                let (a, b) = (parse(la), parse(lb));
                for (name, f) in plain {
                    let expected = f(&a, &b).unwrap();
                    let func = reg.get(&format!("{GEOF_NS}{name}")).unwrap();
                    // Twice: the first call may parse cold (plain relate),
                    // the repeat relates through cached PREPARED sides.
                    for call in ["cold", "warm"] {
                        assert_eq!(
                            func(&[wkt(la), wkt(lb)]).unwrap().to_string(),
                            bool_term(expected),
                            "geof:{name} diverged on {la} vs {lb} ({call} call)"
                        );
                    }
                }
            }
        }
        // Sanity: the prepared path really ran (corpus entries were reused).
        assert!(geom_cache::prepare_count() > 0, "differential never exercised a prepared side");
    }

    #[test]
    fn prepared_relate_patterns_and_errors_equal_unprepared() {
        geom_cache::reset();
        let reg = geof_registry();
        let f = reg.get(&format!("{GEOF_NS}relate")).unwrap();
        let pat = |p: &str| Term::Literal(Literal::new_simple_literal(p));
        let patterns = ["T*****FF*", "FF*FF****", "TFFFTFFFT", "0F2FF1FF2", "T*T***T**"];
        for la in ["POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))", "LINESTRING(-1 -1, 5 5)", "POINT(1 1)"]
        {
            for lb in RELATE_CORPUS {
                let (a, b) = (parse(la), parse(lb));
                for p in patterns {
                    let expected = geof::relate(&a, &b, p).unwrap();
                    for call in ["cold", "warm"] {
                        assert_eq!(
                            f(&[wkt(la), wkt(lb), pat(p)]).unwrap().to_string(),
                            bool_term(expected),
                            "geof:relate({p:?}) diverged on {la} vs {lb} ({call} call)"
                        );
                    }
                }
                // A malformed pattern errors with the plain path's message —
                // on the prepared path too (operands are warm by now).
                let expected_err = geof::relate(&a, &b, "TTT").unwrap_err().to_string();
                assert_eq!(f(&[wkt(la), wkt(lb), pat("TTT")]).unwrap_err(), expected_err);
            }
        }
        assert!(geom_cache::prepare_count() > 0, "relate never exercised a prepared side");
    }

    #[test]
    fn gml_relate_operands_are_prepared_and_result_identical() {
        geom_cache::reset();
        let reg = geof_registry();
        let within = reg.get(&format!("{GEOF_NS}sfWithin")).unwrap();
        let gml_lex = "<gml:Point><gml:pos>-83.38 33.95</gml:pos></gml:Point>";
        let gml = Term::Literal(Literal::new_typed_literal(
            gml_lex,
            NamedNode::new_unchecked(GML_LITERAL),
        ));
        let poly_lex = "POLYGON((-84 33, -83 33, -83 34, -84 34, -84 33))";
        let poly = wkt(poly_lex);
        let expected = geof::sf_within(
            &crate::gml::parse_gml_literal(gml_lex).unwrap(),
            &parse(poly_lex),
        )
        .unwrap();
        assert!(expected, "fixture: the GML point lies inside the polygon");
        for _ in 0..3 {
            assert_eq!(
                within(&[gml.clone(), poly.clone()]).unwrap().to_string(),
                bool_term(expected)
            );
        }
        // Both operands prepared exactly once (on their first warm row) —
        // GML entries participate in the prepared cache like WKT ones.
        assert_eq!(geom_cache::prepare_count(), 2);
    }

    // ---- GeoSPARQL 1.1 accessors (`geof_accessors`, sq-lsp7k) ----
    //
    // The load-bearing invariant: each registered geof:<name> returns EXACTLY
    // what the corresponding crate::metadata pure fn computes for the same
    // geometry — a value where the pure fn defines one, an expression error
    // (never a fabricated integer/boolean/IRI) where it does not (the empty
    // geometry's dimensions, an empty/GEOMETRYCOLLECTION operand's sf: class).
    #[cfg(feature = "geof_accessors")]
    mod geof_accessor_tests {
        use super::*;
        use crate::metadata::{self, lex};

        const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
        const XSD_ANY_URI: &str = "http://www.w3.org/2001/XMLSchema#anyURI";

        /// WKT forms spanning every accessor decision: each simple-features
        /// class, simple + non-simple curves/multipoints, a CRS-prefixed
        /// form, a collection, and the empties (whose integer accessors and
        /// sf: class are UNDEFINED and must error, not fabricate).
        const CORPUS: [&str; 15] = [
            "POINT(1 2)",
            "<http://www.opengis.net/def/crs/EPSG/0/27700> POINT(530000 180000)",
            "LINESTRING(0 0, 1 0, 1 1)",
            "LINESTRING(0 0, 2 2, 0 2, 2 0)", // non-simple bowtie
            "POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))",
            "POLYGON((0 0, 8 0, 8 8, 0 8, 0 0), (2 2, 6 2, 6 6, 2 6, 2 2))",
            "MULTIPOINT((0 0),(1 1))",
            "MULTIPOINT((0 0),(0 0))", // non-simple: coincident pair
            "MULTILINESTRING((0 0, 1 1),(2 2, 3 3))",
            "MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)),((2 2,3 2,3 3,2 3,2 2)))",
            "GEOMETRYCOLLECTION(POINT(0 0), POLYGON((0 0,1 0,1 1,0 1,0 0)))",
            "POINT EMPTY",
            "LINESTRING EMPTY",
            "POLYGON EMPTY",
            "GEOMETRYCOLLECTION EMPTY",
        ];

        #[test]
        fn geof_accessor_integers_match_the_metadata_pure_fns() {
            let reg = geof_registry();
            type LexFn = fn(&str, &str) -> Result<Option<u8>, GeoError>;
            for lexf in CORPUS {
                let m = parse(lexf).metadata();
                let cases: [(&str, Option<u8>, LexFn); 3] = [
                    ("dimension", m.dimension, lex::dimension),
                    ("coordinateDimension", m.coordinate_dimension, lex::coordinate_dimension),
                    ("spatialDimension", m.spatial_dimension, lex::spatial_dimension),
                ];
                for (name, expected, lex_fn) in cases {
                    // The two pure metadata surfaces agree with each other…
                    assert_eq!(
                        lex_fn(lexf, WKT_LITERAL).unwrap(),
                        expected,
                        "metadata() field vs lex helper split on {lexf}"
                    );
                    let f = reg.get(&format!("{GEOF_NS}{name}")).unwrap();
                    match expected {
                        // …and the registry returns exactly the pure fn's
                        // value, typed xsd:integer…
                        Some(v) => {
                            let Ok(Term::Literal(l)) = f(&[wkt(lexf)]) else {
                                panic!("geof:{name}({lexf}) must return a literal")
                            };
                            assert_eq!(l.datatype().as_str(), XSD_INTEGER, "geof:{name}({lexf})");
                            assert_eq!(l.value(), v.to_string(), "geof:{name}({lexf})");
                        }
                        // …or errs where the pure fn defines NO value (the
                        // empty geometry) — never a fabricated integer.
                        None => {
                            let err = f(&[wkt(lexf)]).unwrap_err();
                            assert!(err.contains("unsupported"), "geof:{name}({lexf}): {err}");
                        }
                    }
                }
            }
        }

        #[test]
        fn geof_accessor_is_simple_matches_the_metadata_pure_fn() {
            let reg = geof_registry();
            let f = reg.get(&format!("{GEOF_NS}isSimple")).unwrap();
            let mut saw = [false, false];
            for lexf in CORPUS {
                let expected = metadata::is_simple(&parse(lexf).geometry);
                assert_eq!(lex::is_simple(lexf, WKT_LITERAL).unwrap(), expected);
                assert_eq!(
                    f(&[wkt(lexf)]).unwrap().to_string(),
                    bool_term(expected),
                    "geof:isSimple({lexf})"
                );
                saw[usize::from(expected)] = true;
            }
            assert!(saw[0] && saw[1], "corpus must exercise both isSimple outcomes");
        }

        #[test]
        fn geof_accessor_geometry_type_returns_the_sf_class_iri() {
            let reg = geof_registry();
            let f = reg.get(&format!("{GEOF_NS}geometryType")).unwrap();
            // Differential over the whole corpus: registry == pure fn.
            for lexf in CORPUS {
                match metadata::sf_geometry_type(&parse(lexf).geometry) {
                    Some(iri) => {
                        let Ok(Term::Literal(l)) = f(&[wkt(lexf)]) else {
                            panic!("geof:geometryType({lexf}) must return a literal")
                        };
                        assert_eq!(l.value(), iri, "geof:geometryType({lexf})");
                        assert_eq!(l.datatype().as_str(), XSD_ANY_URI, "geof:geometryType({lexf})");
                        assert!(
                            iri.starts_with(crate::vocab::SF_NS),
                            "sf: class {iri} outside SF_NS"
                        );
                    }
                    None => {
                        let err = f(&[wkt(lexf)]).unwrap_err();
                        assert!(err.contains("unsupported"), "geof:geometryType({lexf}): {err}");
                    }
                }
            }
            // Exact-value pins (a scrambled class mapping turns these red).
            for (lexf, class) in [
                ("POINT(1 2)", "Point"),
                ("LINESTRING(0 0, 1 0, 1 1)", "LineString"),
                ("POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))", "Polygon"),
                ("MULTIPOINT((0 0),(1 1))", "MultiPoint"),
                ("MULTILINESTRING((0 0, 1 1),(2 2, 3 3))", "MultiLineString"),
                ("MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)))", "MultiPolygon"),
            ] {
                let Ok(Term::Literal(l)) = f(&[wkt(lexf)]) else { panic!("literal") };
                assert_eq!(l.value(), format!("http://www.opengis.net/ont/sf#{class}"));
            }
            // No fabricated class: a collection and the empties decline.
            for lexf in [
                "GEOMETRYCOLLECTION(POINT(0 0), POLYGON((0 0,1 0,1 1,0 1,0 0)))",
                "POINT EMPTY",
                "GEOMETRYCOLLECTION EMPTY",
            ] {
                assert!(f(&[wkt(lexf)]).is_err(), "geof:geometryType({lexf}) must err");
            }
        }

        #[test]
        fn geof_accessor_bad_arguments_are_errs() {
            let reg = geof_registry();
            for name in
                ["dimension", "coordinateDimension", "spatialDimension", "isSimple", "geometryType"]
            {
                let f = reg.get(&format!("{GEOF_NS}{name}")).unwrap();
                // Arity: exactly one argument.
                assert!(f(&[]).is_err(), "geof:{name} must enforce unary arity");
                assert!(
                    f(&[wkt("POINT(0 0)"), wkt("POINT(1 1)")]).is_err(),
                    "geof:{name} must reject two arguments"
                );
                // Not a geometry literal (a plain string), and unparsable WKT.
                assert!(
                    f(&[Term::Literal(Literal::new_simple_literal("POINT(1 1)"))]).is_err(),
                    "geof:{name} must reject a plain-string argument"
                );
                assert!(f(&[wkt("PONT(1 1)")]).is_err(), "geof:{name} must reject bad WKT");
            }
        }

        #[test]
        fn geof_accessor_gml_operands_ride_the_same_path() {
            // geom_arg accepts geo:gmlLiteral for the accessors exactly as
            // for every other geof: function.
            let reg = geof_registry();
            let gml = Term::Literal(Literal::new_typed_literal(
                "<gml:Point><gml:pos>-83.38 33.95</gml:pos></gml:Point>",
                NamedNode::new_unchecked(GML_LITERAL),
            ));
            let dim = reg.get(&format!("{GEOF_NS}dimension")).unwrap();
            let Term::Literal(l) = dim(std::slice::from_ref(&gml)).unwrap() else {
                panic!("literal")
            };
            assert_eq!((l.value(), l.datatype().as_str()), ("0", XSD_INTEGER));
            let ty = reg.get(&format!("{GEOF_NS}geometryType")).unwrap();
            let Term::Literal(l) = ty(&[gml]).unwrap() else { panic!("literal") };
            assert_eq!(l.value(), "http://www.opengis.net/ont/sf#Point");
        }
    }
}
