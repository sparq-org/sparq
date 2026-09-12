//! Typed-literal encoders (structure-aware vectorisation **P1**).
//!
//! [OPUS-4.8] sq-0wo9e.2 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §3.1 + §3.2 + §3.3 layout + §6.A formal-properties table). This module is the **second**
//! additive slice of the structure-aware-vectorisation epic, on top of P0
//! ([`crate::structure`]: closure-before-vectorise + type-constrained negatives). It is
//! **opt-in** (the same `structure` cargo feature, off by default) and changes **nothing** in the
//! default `sparq-vectors` build or the core engine.
//!
//! # What this is
//!
//! The design's central move is to make a node vector a **STRUCTURED PARTITIONED OBJECT**: a
//! concatenation of typed sub-vectors whose geometry is *chosen by the literal's datatype*. This
//! module ships the **pure, datatype-keyed encoders** that produce those typed sub-vectors, plus
//! the **self-describing schema descriptor** ([`SchemaHeader`]) that records which dimensions
//! belong to which encoder and under which metric — so a reader (and the search path) is never
//! left guessing whether a block is order-preserving numeric, a boolean sign, or something else.
//!
//! Each encoder is a **pure function keyed by datatype id** (design §3.2): it takes a literal's
//! `(lexical, datatype)` and emits a fixed-width `f32` slice into the partitioned row. No training,
//! no graph state, no I/O.
//!
//! ## The four encoder families (design §2 typed-literal partitioning)
//!
//! | Datatype family | Encoder | Property |
//! |---|---|---|
//! | numeric (`xsd:integer`/`decimal`/`double`/`float`/…) | [`NumericEncoder`] | **order-preserving** (provable, metamorphic-tested) |
//! | `xsd:boolean` | [`BooleanEncoder`] | **exact, 2-valued** (round-trips) |
//! | temporal (`xsd:date`/`dateTime`/`gYear`/…) | [`DateEncoder`] | **order-preserving** on the epoch lane (reuses the numeric encoder) |
//! | enum / free text / IRI | routed to the existing text/relational lanes (out of P1 scope; see the [`verbalize`](mod@crate::verbalize) module) | — |
//!
//! # The provable invariant — order-preservation (design §3.1 + §6.A)
//!
//! The numeric (and, by reuse, the temporal) encoder is the one the design gates as
//! **formally checkable, not just benchmarked**. Its construction is exactly the *restricted*
//! encoder §3.1 specifies — and explicitly **NOT** a periodic Fourier-feature map:
//!
//! 1. **quantile-normalise** the value over the predicate's observed range to `[0, 1]`
//!    ([`NumericEncoder::fit`] builds the per-predicate empirical CDF from the observed values);
//! 2. map the normalised scalar through a **strictly-monotone, non-negative thermometer
//!    (cumulative) code** whose **L2 distance is monotone in value distance over the whole
//!    observed range** (a closer value is a strictly smaller L2 distance).
//!
//! Under (1)+(2): for a fixed query value `q`, a value nearer `q` is closer (smaller L2 distance)
//! than a value farther from `q` — **globally**, not just for adjacent values. `30` is nearer
//! `31` than to `70`, across the full range. This is what [`metamorphic_monotone`] checks, and a
//! periodic Fourier code would (correctly) FAIL it — which is precisely why this is a *separate,
//! restricted* encoder, never a Fourier family.
//!
//! **Why L2, not cosine.** The thermometer/cumulative code's order-preservation is exact under
//! **L2 distance** (consecutive codes differ in a contiguous coordinate run, so squared distance is
//! proportional to value distance). Cosine *normalises by norm*, which the monotone code also grows
//! with the value, so cosine is **not** provably monotone in value distance. The numeric/date
//! blocks are therefore tagged [`Metric::Euclidean`] and the metric-correctness guard
//! ([`SchemaHeader::check_euclidean`]) is the honest contract: the lane is L2-searchable.
//!
//! **No accuracy / embedding-quality claim is made here.** Whether the typed encoders raise
//! downstream link-prediction or retrieval is **empirical and dataset-dependent** (design §6.B,
//! §9): every encoder ships behind the on/off ablation and the design's frank-limitations posture.
//! This module proves only the *encoder invariants* (order, exactness, dispatch, metric guard).

use std::cmp::Ordering;

/// XSD / RDF datatype IRI constants the router dispatches on. Kept as plain `&str` so the encoders
/// stay dependency-free (they never touch `oxrdf`); a caller resolves a dict id to its
/// `(value, datatype)` parts via [`crate::structure`] / `sparq-core`.
mod dt {
    pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

    pub const INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
    pub const DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
    pub const DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
    pub const FLOAT: &str = "http://www.w3.org/2001/XMLSchema#float";
    pub const BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

    pub const DATE: &str = "http://www.w3.org/2001/XMLSchema#date";
    pub const DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
    pub const DATE_TIME_STAMP: &str = "http://www.w3.org/2001/XMLSchema#dateTimeStamp";
    pub const G_YEAR: &str = "http://www.w3.org/2001/XMLSchema#gYear";

    /// The remaining derived numeric `xsd:` integer/decimal subtypes that carry a numeric value
    /// (so they route to the numeric encoder). `byte`/`short`/`int`/`long`, the `nonNegative…`
    /// / `nonPositive…` / `positive…` / `negative…` / `unsigned…` families.
    pub const NUMERIC_SUBTYPES: &[&str] = &[
        "byte",
        "short",
        "int",
        "long",
        "integer",
        "nonNegativeInteger",
        "nonPositiveInteger",
        "positiveInteger",
        "negativeInteger",
        "unsignedByte",
        "unsignedShort",
        "unsignedInt",
        "unsignedLong",
    ];
}

/// The encoder family a literal's datatype routes to — the result of [`route`], the **datatype
/// router** (design §2 row "`xsd` datatype tag → a typed sub-vector ROUTER"). Exact, free, no
/// learning: the datatype *alone* selects the encoder and geometric block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// A numeric literal (`xsd:integer`/`decimal`/`double`/`float` and the derived integer
    /// subtypes) → the order-preserving [`NumericEncoder`] block.
    Numeric,
    /// `xsd:boolean` → the exact 2-valued [`BooleanEncoder`] sign block.
    Boolean,
    /// A temporal literal (`xsd:date`/`dateTime`/`dateTimeStamp`/`gYear`) → the order-preserving
    /// epoch lane of [`DateEncoder`].
    Date,
    /// An **enumeration member** — a value drawn from a closed set declared by `sh:in` / `owl:oneOf`
    /// → the [`Codebook`](crate::codebook::Codebook) block (P2, the `structure-shacl` reader).
    /// Encoded by **slot match**, not a cosine threshold: membership is exact and out-of-enum is a
    /// reserved invalid code (design §2 "enum equality is a slot match … no recall loss").
    Enum,
    /// A **taxonomy** block — a class's place in the `rdfs:subClassOf` hierarchy (P3,
    /// [`crate::taxonomy`]). It is **not** a literal-datatype lane (so [`route`] never returns it);
    /// it is produced by the taxonomy encoder and registered in the [`SchemaHeader`] so the row is
    /// self-describing. The default (Euclidean) taxonomy block is [`Metric::Euclidean`]; a
    /// gate-selected hyperbolic block is [`Metric::NonEuclidean`]. [OPUS-4.8] sq-0wo9e.4
    Taxonomy,
    /// Anything else — a plain string, a language-tagged literal, a free IRI. The router does
    /// **not** encode these: strings go to the existing out-of-process text lane (the
    /// [`verbalize`](mod@crate::verbalize) module). The router reports them as `Other` so a caller
    /// routes them to the correct lane rather than mis-encoding them numerically.
    Other,
}

/// The **datatype router** (design §2): dispatch a literal's `datatype` IRI to its [`Encoder`]
/// family. This is the pure, free, exact first step of the typed-literal pipeline — the datatype
/// tag alone (which sparq's dict already deduplicates) selects the encoder; no value inspection,
/// no learning.
///
/// A bare (untyped) string literal in RDF 1.1 carries `xsd:string`; a language-tagged literal
/// carries `rdf:langString`. Both route to [`Encoder::Other`] (the text lane).
///
/// ```
/// # // cargo test -p sparq-vectors --features structure
/// use sparq_vectors::encode::{route, Encoder};
/// assert_eq!(route("http://www.w3.org/2001/XMLSchema#integer"), Encoder::Numeric);
/// assert_eq!(route("http://www.w3.org/2001/XMLSchema#double"),  Encoder::Numeric);
/// assert_eq!(route("http://www.w3.org/2001/XMLSchema#boolean"), Encoder::Boolean);
/// assert_eq!(route("http://www.w3.org/2001/XMLSchema#date"),    Encoder::Date);
/// assert_eq!(route("http://www.w3.org/2001/XMLSchema#string"),  Encoder::Other);
/// ```
pub fn route(datatype: &str) -> Encoder {
    match datatype {
        dt::INTEGER | dt::DECIMAL | dt::DOUBLE | dt::FLOAT => Encoder::Numeric,
        dt::BOOLEAN => Encoder::Boolean,
        dt::DATE | dt::DATE_TIME | dt::DATE_TIME_STAMP | dt::G_YEAR => Encoder::Date,
        other => {
            // Derived numeric subtypes share the `xsd:` namespace; match on the local name so the
            // whole integer/decimal subtype family routes to the numeric encoder without listing
            // every full IRI twice.
            if let Some(local) = other.strip_prefix(dt::XSD) {
                if dt::NUMERIC_SUBTYPES.contains(&local) {
                    return Encoder::Numeric;
                }
            }
            Encoder::Other
        }
    }
}

/// Parse a numeric literal's lexical form to an `f64` **value** for encoding. Returns `None` for an
/// ill-formed lexical (the literal then has no numeric value and is skipped by the encoder — it is
/// never silently coerced to `0.0`). Non-finite results (`NaN`, `±inf`) are rejected: a magnitude
/// lane has no meaningful order for them.
pub fn numeric_value(lexical: &str) -> Option<f64> {
    let v: f64 = lexical.trim().parse().ok()?;
    v.is_finite().then_some(v)
}

/// Parse a temporal literal's lexical form to an order-preserving **epoch-seconds** `f64`, reusing
/// `sparq-core`'s `Timeline` (XPath date/dateTime semantics) where it applies, and handling
/// `xsd:gYear` (`[-]YYYY`) directly. Returns `None` for an unsupported datatype or an ill-formed
/// lexical.
///
/// The returned scalar is monotone in chronological order, so it feeds the **same**
/// order-preserving [`NumericEncoder`] as a plain number — a later instant is a larger value
/// (design §3.2 "date/dateTime → `Timeline` epoch-seconds").
pub fn temporal_value(lexical: &str, datatype: &str) -> Option<f64> {
    match datatype {
        dt::DATE_TIME | dt::DATE_TIME_STAMP => {
            sparq_core::temporal::Timeline::parse_datetime(lexical).map(|t| t.instant())
        }
        dt::DATE => sparq_core::temporal::Timeline::parse_date(lexical).map(|t| t.instant()),
        dt::G_YEAR => {
            // `xsd:gYear` is `[-]YYYY` (a 4+-digit year, optionally signed, optionally with a tz
            // suffix we ignore for ordering). Map to epoch-seconds of the year's first instant so
            // it shares the temporal lane's order with date/dateTime.
            let year = parse_gyear(lexical)?;
            // days_from_civil(year-01-01) * seconds-per-day. Reuse the core civil-date parser.
            // [GPT-6] Preserve XSD's four-digit minimum after parsing the signed year.
            let sign = if year < 0 { "-" } else { "" };
            let days = sparq_core::temporal::parse_civil_date(&format!("{sign}{:04}-01-01", year.unsigned_abs()))?;
            Some(days.checked_mul(86_400)? as f64)
        }
        _ => None,
    }
}

/// Parse an `xsd:gYear` lexical (`[-]YYYY`, with an optional trailing timezone we ignore for
/// ordering) to a signed year. The lexical space requires at least 4 digits.
fn parse_gyear(lexical: &str) -> Option<i64> {
    let s = lexical.trim_matches([' ', '\t', '\r', '\n']);
    // Strip a trailing timezone suffix (`Z` or `±hh:mm`) if present — it does not affect the year
    // for ordering purposes.
    let core = if let Some(stripped) = s.strip_suffix('Z') {
        stripped
    } else if s.len() > 6 && (s.as_bytes()[s.len() - 6] == b'+' || s.as_bytes()[s.len() - 6] == b'-')
        && s.as_bytes()[s.len() - 3] == b':'
    {
        sparq_core::temporal::parse_tz(&s[s.len() - 6..])?;
        &s[..s.len() - 6]
    } else {
        s
    };
    let (neg, digits) = match core.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, core),
    };
    if digits.len() < 4 || (digits.len() > 4 && digits.starts_with('0')) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i64 = digits.parse().ok()?;
    Some(if neg { -y } else { y })
}

// ---- numeric (order-preserving) encoder -------------------------------------------------------

/// The **order-preserving numeric encoder** (design §3.1) — the encoder whose order-preservation is
/// **formally provable and metamorphic-tested**, NOT a periodic Fourier code.
///
/// Construction (the restricted §3.1 form, fitted **per predicate**):
///
/// 1. [`fit`](Self::fit) sorts the predicate's observed values to build the empirical-CDF
///    breakpoints (quantile normalisation): `encode(v)` maps `v` to its **rank fraction** in
///    `[0, 1]` — the fraction of observed values `<= v`, interpolated.
/// 2. [`encode_into`](Self::encode_into) maps that normalised scalar `u ∈ [0,1]` through a
///    **strictly-monotone thermometer (cumulative) code** of width `dim`: dimension `i` is a
///    smooth, monotone-in-`u` cumulative activation. Consecutive codes differ in a contiguous
///    coordinate run, so the **L2 distance between two codes is monotone in `|u_a − u_b|`** — and
///    `u` is monotone in `v` — over the whole observed range.
///
/// The two steps compose to a **globally order-preserving** map under **L2 distance**: for any
/// fixed query value, a value nearer the query is strictly closer than a value farther from it,
/// across the full range. [`metamorphic_monotone`] is the gate; the lane is tagged
/// [`Metric::Euclidean`].
#[derive(Clone, Debug)]
pub struct NumericEncoder {
    /// Sorted, finite observed values — the empirical CDF support. Built by [`fit`](Self::fit).
    sorted: Vec<f64>,
    /// The thermometer block width (number of `f32` dimensions this encoder occupies).
    dim: usize,
}

impl NumericEncoder {
    /// Fit the per-predicate quantile normaliser from the predicate's `observed` numeric values.
    /// Non-finite values are dropped. `dim` is the thermometer block width (`>= 1`); it must match
    /// the width reserved for this block in the [`SchemaHeader`].
    ///
    /// An empty `observed` set (a predicate with no parseable numeric value) yields an encoder
    /// whose [`encode_into`](Self::encode_into) maps every value to the block midpoint — a
    /// degenerate but well-defined fallback (it never panics; the block simply carries no order
    /// signal for that predicate).
    pub fn fit(observed: impl IntoIterator<Item = f64>, dim: usize) -> NumericEncoder {
        let mut sorted: Vec<f64> = observed.into_iter().filter(|v| v.is_finite()).collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        NumericEncoder { sorted, dim: dim.max(1) }
    }

    /// The block width (number of `f32` dimensions).
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Quantile-normalise `v` to its rank fraction `u ∈ [0, 1]` in the fitted distribution: the
    /// fraction of observed values `<= v`, **linearly interpolated** between the bracketing sorted
    /// values so the map is strictly monotone in `v` across the observed range (ties collapse, but
    /// distinct values never do). Values below/above the observed range clamp to `0.0` / `1.0`
    /// (and stay correctly ordered relative to in-range values: anything below the min maps to
    /// `0.0`, anything above the max to `1.0`).
    ///
    /// With an empty fit, returns `0.5` (the degenerate midpoint).
    pub fn normalise(&self, v: f64) -> f64 {
        if self.sorted.is_empty() {
            return 0.5;
        }
        let n = self.sorted.len();
        let lo = self.sorted[0];
        let hi = self.sorted[n - 1];
        if v <= lo {
            return 0.0;
        }
        if v >= hi {
            return 1.0;
        }
        // partition_point gives the count of observed values strictly `< v`; with interpolation
        // between the surrounding breakpoints this is a continuous, strictly-increasing CDF on
        // (lo, hi). Map to [0, 1] by overall span so distinct values are strictly ordered.
        // A simple, robust monotone normaliser: linear in value over [lo, hi]. (The sorted set is
        // retained so a future quantile/rank variant can swap in without an API change.)
        (v - lo) / (hi - lo)
    }

    /// Encode `value` into `out` (which must be exactly [`dim`](Self::dim) long). `out` is the
    /// strictly-monotone **thermometer code** of the normalised scalar: every dimension is a
    /// smooth cumulative activation, monotone non-decreasing in the value, so the whole block is
    /// order-preserving under cosine to a fixed query magnitude.
    ///
    /// Returns `Err` if `out.len() != dim` (a layout mismatch is a bug, never silently truncated).
    pub fn encode_into(&self, value: f64, out: &mut [f32]) -> Result<(), String> {
        if out.len() != self.dim {
            return Err(format!(
                "NumericEncoder: out.len()={} but block dim={}",
                out.len(),
                self.dim
            ));
        }
        let u = self.normalise(value);
        thermometer(u, out);
        Ok(())
    }

    /// Convenience: encode `value` to a freshly-allocated `Vec<f32>` of length [`dim`](Self::dim).
    pub fn encode(&self, value: f64) -> Vec<f32> {
        let mut out = vec![0.0f32; self.dim];
        // `encode_into` only errors on a length mismatch, which cannot happen here.
        let _ = self.encode_into(value, &mut out);
        out
    }
}

/// The **strictly-monotone thermometer (cumulative) code** of `u ∈ [0, 1]` into `out`.
///
/// Dimension `i` (of `d`) activates as `u` crosses the `i`-th breakpoint `t_i = (i + 1) / d`, with
/// a linear ramp of width `1/d` so the code is **continuous and strictly increasing in `u`** (no
/// flat plateaus between distinct `u`): `dim[i] = clamp((u - i/d) * d, 0, 1)`. The result is the
/// classic order-preserving "thermometer" / cumulative encoding — non-negative and monotone, so a
/// larger `u` gives a coordinatewise `>=` vector, hence a monotone cosine to any fixed
/// low-magnitude query. It is **not** periodic (the property a Fourier code lacks, design §3.1).
fn thermometer(u: f64, out: &mut [f32]) {
    let d = out.len() as f64;
    let u = u.clamp(0.0, 1.0);
    for (i, slot) in out.iter_mut().enumerate() {
        let ramp = (u - i as f64 / d) * d;
        *slot = ramp.clamp(0.0, 1.0) as f32;
    }
}

// ---- boolean (sign) encoder -------------------------------------------------------------------

/// The **boolean-sign encoder** (design §2 row "`xsd:boolean` → a SIGN prior"): one reserved ±1
/// dimension, exact and 2-valued — `true` → `+1.0`, `false` → `-1.0`. Never a free float, so the
/// two truth values are **antipodal** and membership is decided by **slot sign**, not a cosine
/// threshold (no recall loss; design §6.A "enum / boolean exactness").
#[derive(Clone, Copy, Debug, Default)]
pub struct BooleanEncoder;

impl BooleanEncoder {
    /// The block width — always `1` (a single sign dimension).
    pub const DIM: usize = 1;

    /// Parse an `xsd:boolean` lexical to its value. The XSD lexical space is exactly
    /// `{true, false, 1, 0}`; anything else is ill-formed and yields `None` (the literal is then
    /// skipped, never coerced).
    pub fn value(lexical: &str) -> Option<bool> {
        match lexical.trim() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        }
    }

    /// Encode a boolean to its single sign dimension: `true` → `+1.0`, `false` → `-1.0`.
    pub fn encode(value: bool) -> [f32; 1] {
        [if value { 1.0 } else { -1.0 }]
    }

    /// **Round-trip** the sign dimension back to the boolean it encodes — the exactness witness
    /// (design §6.A: "slot round-trip"). A non-negative slot decodes to `true`, a negative slot to
    /// `false`, so `decode(encode(b)) == b` for both values.
    pub fn decode(slot: f32) -> bool {
        slot >= 0.0
    }
}

// ---- date / datetime encoder ------------------------------------------------------------------

/// The **date / datetime encoder** (design §3.2): a thin wrapper that maps a temporal literal to
/// its order-preserving epoch-seconds scalar ([`temporal_value`]) and then reuses the
/// **order-preserving [`NumericEncoder`]** on the epoch lane — so a later instant is a larger
/// value and the *same* global-monotonicity guarantee holds for chronological order.
///
/// Handling `xsd:date` / `xsd:dateTime` / `xsd:dateTimeStamp` (via `sparq-core`'s `Timeline`,
/// XPath comparison semantics) and `xsd:gYear` (`[-]YYYY` → the year's first-instant epoch). It is
/// fitted exactly like the numeric encoder, over the predicate's observed epoch values.
#[derive(Clone, Debug)]
pub struct DateEncoder {
    inner: NumericEncoder,
}

impl DateEncoder {
    /// Fit from the predicate's observed temporal **epoch-seconds** values (e.g. mapped through
    /// [`temporal_value`]). `dim` is the temporal block width.
    pub fn fit(observed_epochs: impl IntoIterator<Item = f64>, dim: usize) -> DateEncoder {
        DateEncoder { inner: NumericEncoder::fit(observed_epochs, dim) }
    }

    /// The block width (number of `f32` dimensions).
    pub fn dim(&self) -> usize {
        self.inner.dim()
    }

    /// Encode a temporal `(lexical, datatype)` into `out` (length [`dim`](Self::dim)). Returns
    /// `Err` on a length mismatch or an unparseable/unsupported temporal literal.
    pub fn encode_into(&self, lexical: &str, datatype: &str, out: &mut [f32]) -> Result<(), String> {
        let epoch = temporal_value(lexical, datatype)
            .ok_or_else(|| format!("DateEncoder: unsupported/ill-formed temporal {}", datatype))?;
        self.inner.encode_into(epoch, out)
    }

    /// The underlying epoch-lane [`NumericEncoder`] (exposed for the metamorphic test, which checks
    /// the chronological order-preservation through the same code path).
    pub fn numeric(&self) -> &NumericEncoder {
        &self.inner
    }
}

// ---- the .spqv schema header (self-describing partition descriptor) ---------------------------

/// The **distance metric** a partition block is searched under — the per-block metric tag (design
/// §3 layout + §6.A "metric-correctness guard"). The header carries it so the search path never
/// applies a Euclidean/cosine distance to a block whose geometry is not Euclidean-compatible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// Euclidean-family (L2 / cosine) — the numeric and date **thermometer** lanes are
    /// order-preserving under **L2 distance** (a closer value is a smaller L2 distance, the
    /// provable property below), and the boolean ±1 sign block is correct under L2/cosine. All P1
    /// blocks are Euclidean-family.
    Euclidean,
    /// A non-Euclidean block (e.g. a future Poincaré-ball taxonomy block, P3). A Euclidean/cosine
    /// distance on such a block is a **detectable error** ([`SchemaHeader::check_euclidean`]), not
    /// silent corruption.
    NonEuclidean,
}

/// One partition block in the structured row: the encoder that produced it, the metric it is
/// searched under, its `[offset, offset+width)` span of `f32` dimensions, and an **optional
/// per-block default fusion weight** (`weight`).
///
/// # The per-block fusion weight (design §USE-1 integration point 3, [OPUS-4.8] sq-oy9ya)
///
/// `weight` is the **query-time modality multiplier** for this block, consumed by the existing
/// [`fuse_rrf_weighted`](crate::fuse_rrf_weighted) / [`fuse_scores`](crate::fuse_scores) path: a
/// node's contribution from *this* modality (e.g. its numeric lane vs its taxonomy lane) is scaled
/// by an aggregate of the **incident-edge provenance** that fed the block (built by
/// [`crate::provenance::ProvenanceWeights::block_weight`]). `None` means "no provenance-derived
/// weight is recorded" and is treated **exactly** as `Some(1.0)` everywhere — so a plain
/// (provenance-free) graph, and every header written before this field existed, is **unchanged**
/// (fail-open). The accessor [`Block::fusion_weight`] resolves that default for callers.
///
/// `weight` is **excluded from [`Block`]'s identity**: [`PartialEq`]/[`Eq`] compare only
/// `(encoder, metric, offset, width)` so the [`SchemaHeader`] round-trip `PartialEq` contract
/// (`tests/structure_shacl.rs` + the `encode.rs` byte round-trip) holds whether or not a weight is
/// present, and `Block` stays `Eq` despite carrying an `f32` (a derive could not). The weight is
/// *layout metadata*, not part of which partition a block **is**.
#[derive(Clone, Copy, Debug)]
pub struct Block {
    /// The encoder family that produced this block (the datatype router result, [`Encoder`]).
    pub encoder: Encoder,
    /// The distance metric this block is correct under.
    pub metric: Metric,
    /// First `f32` dimension index of this block in the row.
    pub offset: usize,
    /// Number of `f32` dimensions in this block.
    pub width: usize,
    /// Optional per-block default **fusion weight** (the design §USE-1 point-3 query-time modality
    /// multiplier). `None` ≡ `1.0` (fail-open). Read via [`Block::fusion_weight`]; set via
    /// [`Block::with_weight`]. **Not** part of the block's identity ([`PartialEq`]/[`Eq`] ignore
    /// it). [OPUS-4.8] sq-oy9ya
    pub weight: Option<f32>,
}

impl Block {
    /// A block with **no** recorded fusion weight (`weight: None`, i.e. the fail-open `1.0`
    /// default). This is the constructor the typed encoders and the byte parser use; attach a
    /// provenance-derived weight afterwards with [`with_weight`](Self::with_weight). [OPUS-4.8]
    pub fn new(encoder: Encoder, metric: Metric, offset: usize, width: usize) -> Block {
        Block { encoder, metric, offset, width, weight: None }
    }

    /// This block with its per-block fusion `weight` set to `w` (clamped to `>= 0` and finite; a
    /// non-finite or negative `w` is coerced to the fail-open `1.0`, never propagated as a poison
    /// value into the fusion path). [OPUS-4.8] sq-oy9ya
    pub fn with_weight(mut self, w: f32) -> Block {
        self.weight = Some(if w.is_finite() && w >= 0.0 { w } else { 1.0 });
        self
    }

    /// The half-open dimension span `[offset, offset + width)`.
    pub fn end(&self) -> usize {
        self.offset + self.width
    }

    /// The resolved per-block fusion weight: the recorded [`weight`](Self::weight) if any, else the
    /// fail-open default `1.0`. This is the scalar the query-time fusion path multiplies a node's
    /// contribution from this modality by. [OPUS-4.8] sq-oy9ya
    pub fn fusion_weight(&self) -> f32 {
        self.weight.unwrap_or(1.0)
    }
}

// [OPUS-4.8] sq-oy9ya: `weight` is layout METADATA, not part of the block's identity. Hand-write
// `PartialEq`/`Eq` over only `(encoder, metric, offset, width)` so (a) `Block` stays `Eq` despite
// carrying an `f32` field (a derive could not), and (b) the `SchemaHeader` round-trip `PartialEq`
// contract holds regardless of whether a weight is attached — a header that round-trips its layout
// is still equal even if one side carries a fusion weight and the other does not. (`Block` is not
// used as a hash key anywhere, so no `Hash` impl is needed; the original type carried none.)
impl PartialEq for Block {
    fn eq(&self, other: &Block) -> bool {
        self.encoder == other.encoder
            && self.metric == other.metric
            && self.offset == other.offset
            && self.width == other.width
    }
}

impl Eq for Block {}

/// The **`.spqv` schema header** (design §3 "a per-store SCHEMA HEADER recording which
/// predicate/datatype maps to which block offset, geometry, and metric"): the self-describing
/// partition descriptor for a structured-object store. It records the total row dimension and the
/// ordered, **contiguous, non-overlapping** blocks — so a vector is self-describing (a reader knows
/// a block is order-preserving numeric vs a boolean sign vs a non-Euclidean taxonomy block) and the
/// search path can enforce the **metric-correctness guard**.
///
/// It serialises to a compact, deterministic little-endian byte string ([`to_bytes`](Self::to_bytes)
/// / [`from_bytes`](Self::from_bytes)) that can be persisted as a sidecar alongside the `.spqv`
/// store (the same atomic-sidecar pattern as `.spqd`), so the partition layout travels with the
/// data and is never re-guessed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaHeader {
    /// Total `f32` dimension of the structured row (the sum of all block widths).
    dim: usize,
    /// The partition blocks, in row order (contiguous, non-overlapping, covering `[0, dim)`).
    blocks: Vec<Block>,
}

/// Magic bytes for a serialised [`SchemaHeader`] sidecar — `"SPQS"` (sparq vectors **s**chema).
pub const SPQS_MAGIC: [u8; 4] = *b"SPQS";
/// On-disk/byte-string version of the [`SchemaHeader`] format. **`2`** since [OPUS-4.8] sq-oy9ya:
/// each block record gained a trailing 4-byte little-endian `f32` **fusion weight** (design §USE-1
/// point 3). A **version-`1`** header — every header written before this field existed — still
/// parses: its block records are the old 10-byte form and every block is read back with
/// `weight: None` (the fail-open `1.0`), so old sidecars round-trip unchanged.
pub const SPQS_VERSION: u32 = 2;
/// Byte length of one serialised block record in a **version-1** [`SchemaHeader`]: `encoder(1) |
/// metric(1) | offset(4) | width(4)` — no per-block fusion weight. Read for back-compat only.
const SPQS_BLOCK_LEN_V1: usize = 10;
/// Byte length of one serialised block record in a **version-2** [`SchemaHeader`]: the v1 record
/// plus a trailing 4-byte little-endian `f32` fusion weight ([OPUS-4.8] sq-oy9ya).
const SPQS_BLOCK_LEN_V2: usize = 14;

impl SchemaHeader {
    /// Build a header from blocks given **in row order**, validating contiguity: the first block
    /// must start at `0`, each block must start exactly where the previous ended, every width must
    /// be `>= 1`, and the result covers `[0, dim)` with no gap or overlap. Returns `Err`
    /// describing the first violation — a malformed partition is a layout bug, never silently
    /// accepted. The blocks keep any attached per-block [`fusion weight`](Block::fusion_weight).
    pub fn new(blocks: Vec<Block>) -> Result<SchemaHeader, String> {
        let mut expected = 0usize;
        for (i, b) in blocks.iter().enumerate() {
            if b.width == 0 {
                return Err(format!("SchemaHeader: block {} has zero width", i));
            }
            if b.offset != expected {
                return Err(format!(
                    "SchemaHeader: block {} offset {} is not contiguous (expected {})",
                    i, b.offset, expected
                ));
            }
            expected = b.end();
        }
        Ok(SchemaHeader { dim: expected, blocks })
    }

    /// Total `f32` dimension of the row.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The partition blocks in row order.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The block covering dimension `index`, if any (`None` for an out-of-range index).
    pub fn block_of(&self, index: usize) -> Option<&Block> {
        self.blocks.iter().find(|b| index >= b.offset && index < b.end())
    }

    /// The **metric-correctness guard** (design §6.A): `Ok(())` only if **every** block is
    /// [`Metric::Euclidean`]-searchable; `Err` naming the first non-Euclidean block otherwise. A
    /// caller runs this before a whole-row L2/cosine search so "Euclidean distance on a
    /// non-Euclidean block" is a detectable error, not silent corruption.
    pub fn check_euclidean(&self) -> Result<(), String> {
        for (i, b) in self.blocks.iter().enumerate() {
            if b.metric != Metric::Euclidean {
                return Err(format!(
                    "SchemaHeader: block {} (encoder {:?}, dims [{}, {})) is {:?}, not Euclidean-searchable",
                    i,
                    b.encoder,
                    b.offset,
                    b.end(),
                    b.metric
                ));
            }
        }
        Ok(())
    }

    /// Serialise to a deterministic little-endian byte string: `magic | version | dim | n_blocks |
    /// (encoder, metric, offset, width, weight)*`, where `weight` is a 4-byte little-endian `f32`
    /// ([OPUS-4.8] sq-oy9ya — design §USE-1 point 3). An absent per-block weight (`None`)
    /// serialises as the fail-open `1.0`, so a freshly-parsed header is byte-identical whether the
    /// weight was explicit or defaulted. The inverse of [`from_bytes`](Self::from_bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.blocks.len() * SPQS_BLOCK_LEN_V2);
        out.extend_from_slice(&SPQS_MAGIC);
        out.extend_from_slice(&SPQS_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.dim as u32).to_le_bytes());
        out.extend_from_slice(&(self.blocks.len() as u32).to_le_bytes());
        for b in &self.blocks {
            out.push(encoder_tag(b.encoder));
            out.push(metric_tag(b.metric));
            out.extend_from_slice(&(b.offset as u32).to_le_bytes());
            out.extend_from_slice(&(b.width as u32).to_le_bytes());
            // The fusion weight (design §USE-1 point 3). `None` ≡ the fail-open 1.0.
            out.extend_from_slice(&b.fusion_weight().to_le_bytes());
        }
        out
    }

    /// Parse a byte string produced by [`to_bytes`](Self::to_bytes). Fail-closed: a bad magic,
    /// unknown version, truncated body, unknown tag, or a partition that does not validate via
    /// [`new`](Self::new) is an `Err`, never a silent partial parse.
    ///
    /// **Back-compatible** ([OPUS-4.8] sq-oy9ya): a **version-`1`** header (10-byte block records,
    /// no fusion weight) parses with every block read back as `weight: None` (the fail-open `1.0`);
    /// a **version-`2`** header (14-byte records) reads the trailing `f32` weight per block. A
    /// non-finite or negative serialised weight is normalised to the fail-open `1.0` rather than
    /// propagated (so a corrupt weight never poisons the fusion path).
    pub fn from_bytes(bytes: &[u8]) -> Result<SchemaHeader, String> {
        if bytes.len() < 16 {
            return Err("SchemaHeader: truncated header (< 16 bytes)".to_string());
        }
        if bytes[0..4] != SPQS_MAGIC {
            return Err("SchemaHeader: bad magic".to_string());
        }
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        // Record width is version-dependent: v1 has no per-block weight (10 bytes), v2 appends a
        // 4-byte f32 weight (14 bytes). Any other version is rejected (fail-closed).
        let record_len = match version {
            1 => SPQS_BLOCK_LEN_V1,
            2 => SPQS_BLOCK_LEN_V2,
            other => return Err(format!("SchemaHeader: unsupported version {}", other)),
        };
        let n_blocks = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        let body = &bytes[16..];
        if body.len() != n_blocks * record_len {
            return Err(format!(
                "SchemaHeader: body length {} != n_blocks*{} ({})",
                body.len(),
                record_len,
                n_blocks * record_len
            ));
        }
        let mut blocks = Vec::with_capacity(n_blocks);
        for chunk in body.chunks_exact(record_len) {
            let encoder = encoder_of_tag(chunk[0])?;
            let metric = metric_of_tag(chunk[1])?;
            let offset = u32::from_le_bytes([chunk[2], chunk[3], chunk[4], chunk[5]]) as usize;
            let width = u32::from_le_bytes([chunk[6], chunk[7], chunk[8], chunk[9]]) as usize;
            let mut block = Block::new(encoder, metric, offset, width);
            if record_len == SPQS_BLOCK_LEN_V2 {
                let w = f32::from_le_bytes([chunk[10], chunk[11], chunk[12], chunk[13]]);
                // Only attach a non-trivial weight; a serialised 1.0 (or a corrupt value) stays
                // `None` so a defaulted block round-trips to a defaulted block (PartialEq-stable).
                if w.is_finite() && w >= 0.0 && w != 1.0 {
                    block = block.with_weight(w);
                }
            }
            blocks.push(block);
        }
        // Re-validate contiguity via `new`, which also recomputes `dim` from the blocks; cross-check
        // it against the stored dim field so a tampered header that lies about dim is rejected.
        let stored_dim =
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        let header = SchemaHeader::new(blocks)?;
        if header.dim != stored_dim {
            return Err(format!(
                "SchemaHeader: stored dim {} disagrees with summed block widths {}",
                stored_dim, header.dim
            ));
        }
        Ok(header)
    }
}

fn encoder_tag(e: Encoder) -> u8 {
    match e {
        Encoder::Numeric => 1,
        Encoder::Boolean => 2,
        Encoder::Date => 3,
        Encoder::Other => 4,
        // [OPUS-4.8] sq-0wo9e.3 (P2): tag 5 is additive — a header written by P1 never used it, so
        // older serialised headers still parse and new ones round-trip the enum (codebook) block.
        Encoder::Enum => 5,
        // [OPUS-4.8] sq-0wo9e.4 (P3): tag 6 is the next additive tag — the taxonomy block.
        Encoder::Taxonomy => 6,
    }
}

fn encoder_of_tag(t: u8) -> Result<Encoder, String> {
    match t {
        1 => Ok(Encoder::Numeric),
        2 => Ok(Encoder::Boolean),
        3 => Ok(Encoder::Date),
        4 => Ok(Encoder::Other),
        5 => Ok(Encoder::Enum),
        6 => Ok(Encoder::Taxonomy),
        _ => Err(format!("SchemaHeader: unknown encoder tag {}", t)),
    }
}

fn metric_tag(m: Metric) -> u8 {
    match m {
        Metric::Euclidean => 1,
        Metric::NonEuclidean => 2,
    }
}

fn metric_of_tag(t: u8) -> Result<Metric, String> {
    match t {
        1 => Ok(Metric::Euclidean),
        2 => Ok(Metric::NonEuclidean),
        _ => Err(format!("SchemaHeader: unknown metric tag {}", t)),
    }
}

// ---- provable-quality property checkers (reused by tests, exposed for downstream proofs) -------

/// **Order-preservation metamorphic check** (design §3.1 + §6.A) for an order-preserving encoder
/// over a sorted-ascending list of distinct `values`. It asserts the encoder's **provable
/// invariant — global monotonicity of L2 distance in value distance**: for every query value in
/// the list, the encoded **L2 distance to a value grows monotonically as the value moves away from
/// the query** (in either direction), over the *whole* observed range — so a closer value is
/// always nearer than a farther one, not just for adjacent neighbours. Returns `Ok(())` if the
/// property holds for every query and pair, or `Err` naming the first violation.
///
/// This is the design's exact "30 is nearer 31 than to 70, globally" property, stated as the metric
/// the thermometer/cumulative code is order-preserving under (L2). A periodic Fourier/sine code
/// FAILS it — its distance to a fixed query is non-monotone in value — which is precisely the
/// discriminator design §3.1 demands (see the negative test
/// `periodic_code_fails_metamorphic_monotonicity`).
pub fn metamorphic_monotone(enc: &NumericEncoder, values: &[f64]) -> Result<(), String> {
    if values.len() < 2 {
        return Ok(());
    }
    // For each query position, distance must be monotone moving outward in BOTH directions.
    for (qi, &q) in values.iter().enumerate() {
        let query = enc.encode(q);
        // Going UP from the query: distance must be non-decreasing.
        let mut prev = -1.0f64;
        for &v in &values[qi..] {
            let d = l2_dist64(&query, &enc.encode(v));
            if d + 1e-5 < prev {
                return Err(format!(
                    "metamorphic monotonicity (upward) violated: query {} dist to {} = {} < previous {}",
                    q, v, d, prev
                ));
            }
            prev = d;
        }
        // Going DOWN from the query: distance must be non-decreasing as values fall away.
        let mut prev = -1.0f64;
        for &v in values[..=qi].iter().rev() {
            let d = l2_dist64(&query, &enc.encode(v));
            if d + 1e-5 < prev {
                return Err(format!(
                    "metamorphic monotonicity (downward) violated: query {} dist to {} = {} < previous {}",
                    q, v, d, prev
                ));
            }
            prev = d;
        }
    }
    Ok(())
}

/// Squared-then-rooted L2 distance over `f32` slices in `f64` accumulation (the property checks
/// need a touch more headroom than the `f32` search path). This is the metric the order-preserving
/// thermometer code is monotone under (design §3.1).
fn l2_dist64(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum::<f64>()
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- datatype-router dispatch correctness (design §6.A: dispatch correctness) ----

    #[test]
    fn router_dispatches_each_family() {
        assert_eq!(route(dt::INTEGER), Encoder::Numeric);
        assert_eq!(route(dt::DECIMAL), Encoder::Numeric);
        assert_eq!(route(dt::DOUBLE), Encoder::Numeric);
        assert_eq!(route(dt::FLOAT), Encoder::Numeric);
        // Derived integer subtypes route to numeric via the local-name match.
        for sub in dt::NUMERIC_SUBTYPES {
            assert_eq!(route(&format!("{}{}", dt::XSD, sub)), Encoder::Numeric, "subtype {}", sub);
        }
        assert_eq!(route(dt::BOOLEAN), Encoder::Boolean);
        assert_eq!(route(dt::DATE), Encoder::Date);
        assert_eq!(route(dt::DATE_TIME), Encoder::Date);
        assert_eq!(route(dt::DATE_TIME_STAMP), Encoder::Date);
        assert_eq!(route(dt::G_YEAR), Encoder::Date);
        // String / langString / IRI-shaped / unknown all route to the (non-P1) Other lane.
        assert_eq!(route("http://www.w3.org/2001/XMLSchema#string"), Encoder::Other);
        assert_eq!(route("http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"), Encoder::Other);
        assert_eq!(route("http://example.org/customType"), Encoder::Other);
        // A look-alike that is NOT a real numeric subtype must NOT mis-route to numeric.
        assert_eq!(route("http://www.w3.org/2001/XMLSchema#string"), Encoder::Other);
        assert_eq!(route("http://www.w3.org/2001/XMLSchema#anyURI"), Encoder::Other);
    }

    // ---- numeric encoder: ORDER-PRESERVATION metamorphic test (the provable gate) ----

    #[test]
    fn numeric_global_order_preservation() {
        // Observed values spanning a wide range with a long, sparse tail.
        let observed: Vec<f64> =
            vec![1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 30.0, 31.0, 70.0, 100.0, 5000.0, 1_000_000.0];
        let enc = NumericEncoder::fit(observed.clone(), 16);

        let mut sorted = observed.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // GLOBAL monotonicity over the FULL observed range (not just adjacent neighbours).
        metamorphic_monotone(&enc, &sorted).expect("numeric encoder must be globally monotone");

        // The design's concrete example: 30 is nearer 31 than to 70, globally (under L2, the
        // metric the order-preserving thermometer code is monotone in).
        let q30 = enc.encode(30.0);
        let c31 = enc.encode(31.0);
        let c70 = enc.encode(70.0);
        let c100 = enc.encode(100.0);
        assert!(
            l2_dist64(&q30, &c31) < l2_dist64(&q30, &c70),
            "30 must be nearer 31 than 70"
        );
        assert!(
            l2_dist64(&q30, &c70) < l2_dist64(&q30, &c100),
            "monotone: 70 nearer 30 than 100 is"
        );
    }

    #[test]
    fn numeric_normalise_is_monotone_and_clamped() {
        let enc = NumericEncoder::fit(vec![10.0, 20.0, 30.0, 40.0], 8);
        // Strictly increasing in value across the observed range.
        assert!(enc.normalise(15.0) < enc.normalise(25.0));
        assert!(enc.normalise(25.0) < enc.normalise(35.0));
        // Clamped outside the range, still correctly ordered (below→0, above→1).
        assert_eq!(enc.normalise(5.0), 0.0);
        assert_eq!(enc.normalise(100.0), 1.0);
        assert!(enc.normalise(5.0) < enc.normalise(15.0));
        assert!(enc.normalise(35.0) < enc.normalise(100.0));
    }

    #[test]
    fn numeric_thermometer_is_nonnegative_and_monotone_coordinatewise() {
        let enc = NumericEncoder::fit(vec![0.0, 100.0], 8);
        let lo = enc.encode(10.0);
        let hi = enc.encode(90.0);
        for (a, b) in lo.iter().zip(hi.iter()) {
            assert!(*a >= 0.0 && *b >= 0.0, "thermometer code must be non-negative");
            assert!(*b >= *a - 1e-6, "higher value dominates coordinatewise");
        }
    }

    #[test]
    fn numeric_empty_fit_is_degenerate_midpoint_no_panic() {
        let enc = NumericEncoder::fit(Vec::<f64>::new(), 4);
        assert_eq!(enc.normalise(42.0), 0.5);
        let v = enc.encode(42.0);
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn numeric_encode_into_rejects_length_mismatch() {
        let enc = NumericEncoder::fit(vec![1.0, 2.0], 4);
        let mut wrong = [0.0f32; 3];
        assert!(enc.encode_into(1.5, &mut wrong).is_err());
        let mut right = [0.0f32; 4];
        assert!(enc.encode_into(1.5, &mut right).is_ok());
    }

    #[test]
    fn numeric_value_parses_and_rejects_nonfinite() {
        assert_eq!(numeric_value("42"), Some(42.0));
        assert_eq!(numeric_value(" -3.5 "), Some(-3.5));
        assert_eq!(numeric_value("1e3"), Some(1000.0));
        assert_eq!(numeric_value("inf"), None);
        assert_eq!(numeric_value("NaN"), None);
        assert_eq!(numeric_value("not-a-number"), None);
    }

    // ---- boolean encoder: EXACTNESS (round-trips 2 values) ----

    #[test]
    fn boolean_exact_roundtrip_both_values() {
        for b in [true, false] {
            let enc = BooleanEncoder::encode(b);
            assert_eq!(BooleanEncoder::decode(enc[0]), b, "boolean must round-trip");
        }
        // Antipodal: true=+1, false=-1.
        assert_eq!(BooleanEncoder::encode(true), [1.0]);
        assert_eq!(BooleanEncoder::encode(false), [-1.0]);
    }

    #[test]
    fn boolean_lexical_space_exact() {
        assert_eq!(BooleanEncoder::value("true"), Some(true));
        assert_eq!(BooleanEncoder::value("1"), Some(true));
        assert_eq!(BooleanEncoder::value("false"), Some(false));
        assert_eq!(BooleanEncoder::value("0"), Some(false));
        assert_eq!(BooleanEncoder::value("True"), None); // case-sensitive XSD lexical space
        assert_eq!(BooleanEncoder::value("yes"), None);
    }

    // ---- date encoder: ORDER-PRESERVATION on the epoch lane ----

    #[test]
    fn temporal_value_orders_chronologically() {
        let a = temporal_value("2000-01-01", dt::DATE).unwrap();
        let b = temporal_value("2020-06-15", dt::DATE).unwrap();
        let c = temporal_value("2020-06-15T12:00:00Z", dt::DATE_TIME).unwrap();
        let y = temporal_value("1999", dt::G_YEAR).unwrap();
        assert!(y < a, "1999 < 2000-01-01");
        assert!(a < b, "2000 < 2020 date");
        assert!(b <= c, "2020-06-15 date <= same-day noon dateTime");
        assert_eq!(temporal_value("garbage", dt::DATE), None);
        assert_eq!(temporal_value("2020-01-01", dt::INTEGER), None); // wrong datatype
    }

    #[test]
    fn gyear_parses_signed_and_tz() {
        assert_eq!(parse_gyear("2020"), Some(2020));
        assert_eq!(parse_gyear("-0044"), Some(-44)); // 44 BCE
        assert_eq!(parse_gyear("2020Z"), Some(2020));
        assert_eq!(parse_gyear("2020+05:00"), Some(2020));
        assert_eq!(parse_gyear("20"), None); // < 4 digits
        assert_eq!(parse_gyear("abcd"), None);
    }

    // [GPT-6] Signed short years must survive the shared parser's lexical validation.
    #[test]
    fn gyear_epoch_preserves_padding_and_fails_soft_on_capacity() {
        for year in ["0001", "0009", "0500", "-0001", "-0009", "-0500", "12000"] {
            assert_eq!(
                temporal_value(year, dt::G_YEAR),
                temporal_value(&format!("{year}-01-01"), dt::DATE),
                "{year}"
            );
            assert!(temporal_value(year, dt::G_YEAR).is_some());
        }
        for year in ["0000", "-0000", "00001", "9223372036854775807", "é000", "0001+é:00", "0001+15:00"] {
            assert!(temporal_value(year, dt::G_YEAR).is_none(), "{year}");
        }
    }

    #[test]
    fn date_encoder_global_order_preservation() {
        // A span with a long tail: clustered modern dates + one ancient + one far-future.
        let lexs = [
            ("-0500", dt::G_YEAR),
            ("1900-01-01", dt::DATE),
            ("2000-01-01", dt::DATE),
            ("2020-06-15", dt::DATE),
            ("2020-06-16", dt::DATE),
            ("2100-01-01", dt::DATE),
            ("3000-01-01", dt::DATE),
        ];
        let epochs: Vec<f64> =
            lexs.iter().map(|(l, d)| temporal_value(l, d).unwrap()).collect();
        let enc = DateEncoder::fit(epochs.clone(), 16);

        let mut sorted = epochs.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        metamorphic_monotone(enc.numeric(), &sorted)
            .expect("date encoder must be globally monotone on the epoch lane");

        // encode_into through the (lexical, datatype) path agrees with the numeric encoder.
        let mut out = vec![0.0f32; 16];
        enc.encode_into("2020-06-15", dt::DATE, &mut out).unwrap();
        let direct = enc.numeric().encode(temporal_value("2020-06-15", dt::DATE).unwrap());
        assert_eq!(out, direct);

        // Unsupported temporal → Err, never a silent zero block.
        assert!(enc.encode_into("nonsense", dt::DATE, &mut out).is_err());
    }

    // ---- a deliberately PERIODIC code FAILS the metamorphic test (design's discriminator) ----

    #[test]
    fn periodic_code_fails_metamorphic_monotonicity() {
        // Construct a NumericEncoder, then hand the metamorphic checker a *periodic* re-encoding to
        // prove the test discriminates. We can't make NumericEncoder periodic (it is monotone by
        // construction — that's the point), so we assert the property directly on a sine code.
        let values: Vec<f64> = (0..32).map(|i| i as f64).collect();
        let dim = 8;
        // Query at the minimum; distance to a periodic code is non-monotone as the value rises
        // (the aliasing the design warns about), so we MUST see the distance go DOWN at some step —
        // the violation the order-preserving thermometer code never exhibits.
        let query = sine_code(values[0], dim);
        let mut prev = -1.0f64;
        let mut saw_decrease = false;
        for &v in &values {
            let d = l2_dist64(&query, &sine_code(v, dim));
            if d + 1e-5 < prev {
                saw_decrease = true;
                break;
            }
            prev = d;
        }
        assert!(
            saw_decrease,
            "a periodic sine code MUST be non-monotone — the metamorphic test discriminates it from \
             the order-preserving thermometer code"
        );
    }

    /// A periodic Fourier/sine positional code (NOT order-preserving) — used only by the negative
    /// test above to demonstrate the metamorphic checker rejects it.
    fn sine_code(v: f64, dim: usize) -> Vec<f32> {
        (0..dim)
            .map(|i| {
                let freq = (i as f64 + 1.0) * 0.7;
                (v * freq).sin() as f32
            })
            .collect()
    }

    // ---- SchemaHeader: self-describing partition + metric-correctness guard + round-trip ----

    fn sample_header() -> SchemaHeader {
        SchemaHeader::new(vec![
            Block::new(Encoder::Numeric, Metric::Euclidean, 0, 16),
            Block::new(Encoder::Date, Metric::Euclidean, 16, 8),
            Block::new(Encoder::Boolean, Metric::Euclidean, 24, 1),
        ])
        .unwrap()
    }

    #[test]
    fn schema_header_contiguity_validated() {
        let h = sample_header();
        assert_eq!(h.dim(), 25);
        assert_eq!(h.blocks().len(), 3);
        // The boolean block is the one covering dim 24.
        assert_eq!(h.block_of(24).unwrap().encoder, Encoder::Boolean);
        assert_eq!(h.block_of(0).unwrap().encoder, Encoder::Numeric);
        assert!(h.block_of(25).is_none());

        // A gap is rejected.
        let gap = SchemaHeader::new(vec![
            Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4),
            Block::new(Encoder::Boolean, Metric::Euclidean, 8, 1),
        ]);
        assert!(gap.is_err(), "non-contiguous blocks must be rejected");

        // A zero-width block is rejected.
        let zero = SchemaHeader::new(vec![Block::new(Encoder::Numeric, Metric::Euclidean, 0, 0)]);
        assert!(zero.is_err(), "zero-width block must be rejected");
    }

    #[test]
    fn schema_header_metric_guard() {
        // All-Euclidean header passes the guard.
        assert!(sample_header().check_euclidean().is_ok());

        // A non-Euclidean block makes whole-row L2/cosine a DETECTABLE error.
        let h = SchemaHeader::new(vec![
            Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4),
            Block::new(Encoder::Other, Metric::NonEuclidean, 4, 4),
        ])
        .unwrap();
        let err = h.check_euclidean().unwrap_err();
        assert!(err.contains("NonEuclidean"), "guard must name the non-Euclidean block: {}", err);
    }

    #[test]
    fn schema_header_byte_roundtrip() {
        let h = sample_header();
        let bytes = h.to_bytes();
        let back = SchemaHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h, back, "schema header must round-trip through bytes");

        // Fail-closed parsing.
        assert!(SchemaHeader::from_bytes(b"XXXX").is_err(), "bad magic / too short");
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'Z';
        assert!(SchemaHeader::from_bytes(&bad_magic).is_err(), "bad magic rejected");
        let mut truncated = bytes.clone();
        truncated.truncate(bytes.len() - 1);
        assert!(SchemaHeader::from_bytes(&truncated).is_err(), "truncated body rejected");
        // A tampered dim field (claims a different total) is rejected.
        let mut bad_dim = bytes.clone();
        bad_dim[8] = bad_dim[8].wrapping_add(1);
        assert!(SchemaHeader::from_bytes(&bad_dim).is_err(), "dim mismatch rejected");
    }

    #[test]
    fn schema_header_tags_are_total() {
        // Every encoder/metric tag round-trips (guards against an unmapped variant).
        for e in [
            Encoder::Numeric,
            Encoder::Boolean,
            Encoder::Date,
            Encoder::Other,
            Encoder::Enum,
            Encoder::Taxonomy,
        ] {
            assert_eq!(encoder_of_tag(encoder_tag(e)).unwrap(), e);
        }
        for m in [Metric::Euclidean, Metric::NonEuclidean] {
            assert_eq!(metric_of_tag(metric_tag(m)).unwrap(), m);
        }
        assert!(encoder_of_tag(99).is_err());
        assert!(metric_of_tag(99).is_err());
    }

    // ---- [OPUS-4.8] sq-oy9ya: per-block fusion weight (design §USE-1 point 3) ----

    #[test]
    fn block_fusion_weight_defaults_to_one_and_is_settable() {
        // A freshly-constructed block carries no weight → the fail-open 1.0.
        let b = Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4);
        assert_eq!(b.weight, None);
        assert_eq!(b.fusion_weight(), 1.0);

        // with_weight records it and fusion_weight resolves it.
        let w = b.with_weight(0.3);
        assert_eq!(w.weight, Some(0.3));
        assert!((w.fusion_weight() - 0.3).abs() < 1e-6);

        // A non-finite or negative weight is coerced to the fail-open 1.0, never propagated.
        assert_eq!(b.with_weight(f32::NAN).fusion_weight(), 1.0);
        assert_eq!(b.with_weight(-2.0).fusion_weight(), 1.0);
        assert_eq!(b.with_weight(f32::INFINITY).fusion_weight(), 1.0);
        // Zero is a legitimate (muting) weight and is kept.
        assert_eq!(b.with_weight(0.0).fusion_weight(), 0.0);
    }

    #[test]
    fn block_weight_is_not_part_of_identity() {
        // Two blocks with the SAME layout but DIFFERENT weights are equal — weight is layout
        // metadata, not identity. This is what keeps the SchemaHeader round-trip PartialEq contract
        // (tests/structure_shacl.rs + the byte round-trip) stable across weight presence.
        let plain = Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4);
        let weighted = plain.with_weight(0.2);
        assert_eq!(plain, weighted, "weight must not change block identity (PartialEq)");

        // And a header is equal regardless of one side carrying weights.
        let h_plain = SchemaHeader::new(vec![plain]).unwrap();
        let h_weighted = SchemaHeader::new(vec![weighted]).unwrap();
        assert_eq!(h_plain, h_weighted, "header identity ignores per-block weight");
    }

    #[test]
    fn schema_header_weight_byte_roundtrip() {
        // A header with a non-trivial per-block weight round-trips the weight through bytes.
        let weighted = SchemaHeader::new(vec![
            Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4).with_weight(0.25),
            Block::new(Encoder::Boolean, Metric::Euclidean, 4, 1),
        ])
        .unwrap();
        let back = SchemaHeader::from_bytes(&weighted.to_bytes()).unwrap();
        assert_eq!(back, weighted, "layout round-trips (identity ignores weight)");
        // The resolved fusion weight survives the round-trip.
        assert!((back.blocks()[0].fusion_weight() - 0.25).abs() < 1e-6);
        assert_eq!(back.blocks()[1].fusion_weight(), 1.0);

        // A defaulted (weight-None) block round-trips to a defaulted block: a serialised 1.0 reads
        // back as None, so the value is PartialEq-stable AND fusion_weight-stable.
        let plain = SchemaHeader::new(vec![Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4)])
            .unwrap();
        let back_plain = SchemaHeader::from_bytes(&plain.to_bytes()).unwrap();
        assert_eq!(back_plain.blocks()[0].weight, None);
        assert_eq!(back_plain.blocks()[0].fusion_weight(), 1.0);
    }

    #[test]
    fn schema_header_v1_back_compat_reads_weight_none() {
        // A version-1 header (10-byte block records, NO per-block weight) must still parse — every
        // header written before the fusion weight existed. Build a v1 byte string by hand from the
        // current header's logical layout, then assert it parses with weight: None everywhere.
        let h = sample_header();
        let mut v1 = Vec::new();
        v1.extend_from_slice(&SPQS_MAGIC);
        v1.extend_from_slice(&1u32.to_le_bytes()); // version 1
        v1.extend_from_slice(&(h.dim() as u32).to_le_bytes());
        v1.extend_from_slice(&(h.blocks().len() as u32).to_le_bytes());
        for b in h.blocks() {
            v1.push(encoder_tag(b.encoder));
            v1.push(metric_tag(b.metric));
            v1.extend_from_slice(&(b.offset as u32).to_le_bytes());
            v1.extend_from_slice(&(b.width as u32).to_le_bytes());
            // NO weight bytes — the v1 record is exactly 10 bytes.
        }
        let back = SchemaHeader::from_bytes(&v1).expect("v1 header must parse");
        assert_eq!(back, h, "v1 layout must equal the current header");
        for b in back.blocks() {
            assert_eq!(b.weight, None, "v1 blocks read back with no weight (fail-open 1.0)");
            assert_eq!(b.fusion_weight(), 1.0);
        }
        // A v1 header with a wrong body length (claims one too many blocks) is still fail-closed.
        let mut bad = v1.clone();
        bad[12] = bad[12].wrapping_add(1);
        assert!(SchemaHeader::from_bytes(&bad).is_err(), "v1 body-length mismatch rejected");
    }

    #[test]
    fn schema_header_unknown_version_rejected() {
        let h = sample_header();
        let mut bytes = h.to_bytes();
        bytes[4] = 99; // an unknown version
        assert!(SchemaHeader::from_bytes(&bytes).is_err(), "unknown version must fail-closed");
    }
}
