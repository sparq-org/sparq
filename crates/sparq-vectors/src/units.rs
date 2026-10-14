//! QUDT unit-normalisation — convert a unit-annotated magnitude to its **canonical SI value**
//! *before* the order-preserving numeric encoder, so `1000 m` and `1 km` share a code
//! (structure-aware vectorisation **P2**).
//!
//! sq-0wo9e.3 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §2 row "QUDT / unit annotations → UNIT-NORMALISE-BEFORE-MAGNITUDE" + §3.1 numeric encoder).
//! Part of the **second** structure-aware slice (P2), on top of P1
//! ([`crate::encode`]: the typed-literal encoders). It is **opt-in** (the same `structure` cargo
//! feature, off by default — no new dependency) and changes **nothing** in the default
//! `sparq-vectors` build or the core engine.
//!
//! # The problem this solves
//!
//! A magnitude is only meaningful *with* its unit. Two literals `"1000"^^…` annotated `qudt:unit
//! unit:M` (metre) and `"1"^^…` annotated `unit:KiloM` (kilometre) denote the **same** physical
//! length, but the raw numbers `1000` and `1` would land at opposite ends of the
//! order-preserving numeric block ([`crate::encode::NumericEncoder`]) — the order-preservation
//! guarantee is then over *lexical* magnitudes, not *physical* ones, which is wrong.
//!
//! The fix (design §2): **normalise to the canonical SI base unit of the quantity kind first**,
//! then feed the canonical value to the numeric encoder. After normalisation both literals are
//! `1000` metres, so they encode identically and order-preservation holds over physical magnitude.
//!
//! # What is, and is NOT, claimed
//!
//! - **Provable (and tested):** the conversion is an **affine map** `canonical = lexical * factor
//!   + offset` with a constant `(factor, offset)` per unit; two unit/value pairs that denote the
//!   same physical quantity map to the **same** canonical value (exactly for affine-exact units,
//!   within f64 rounding otherwise). [`same_quantity_normalises_equal`] is the gate.
//! - **Fail-closed / SHACL-detectable mismatch (design §2 "unit mismatch is SHACL-detectable,
//!   not silent noise"):** [`normalise`] returns the **quantity kind** alongside the value, so a
//!   caller routes each quantity kind to its **own** numeric block (a length is never compared to a
//!   mass). An **unknown** unit IRI yields `None` — the magnitude is then skipped, never silently
//!   treated as dimensionless; and mixing two different [`QuantityKind`]s in one block is a
//!   caller-detectable error, exactly as a `sh:datatype`/unit mismatch is for SHACL.
//! - **NOT claimed:** completeness of the bundled table. This is a **curated, deliberately small**
//!   subset of QUDT (the common SI + a few customary units), not the full QUDT ontology. An
//!   absent unit fails closed (`None`); extending the table is additive. No accuracy/embedding
//!   claim is made — this only makes the *input* to the numeric encoder physically comparable.

/// The QUDT `unit:` vocabulary namespace (the curated subset below uses these IRIs). The full
/// vocabulary is much larger; this crate bundles only the common units (see the module docs).
pub const QUDT_UNIT_NS: &str = "http://qudt.org/vocab/unit/";

/// The physical **quantity kind** a unit measures — the *routing slot* of design §2 ("quantity-kind
/// is a routing slot"). Two values are only physically comparable (and may share a numeric block)
/// when their [`QuantityKind`] matches; the canonical SI base unit is fixed per kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuantityKind {
    /// Length — canonical base **metre** (`m`).
    Length,
    /// Mass — canonical base **kilogram** (`kg`).
    Mass,
    /// Time / duration — canonical base **second** (`s`).
    Time,
    /// Thermodynamic temperature — canonical base **kelvin** (`K`) (affine for °C/°F).
    Temperature,
    /// Plane angle — canonical base **radian** (`rad`).
    Angle,
    /// Data / information size — canonical base **byte** (`B`).
    DataSize,
}

impl QuantityKind {
    /// The canonical SI base unit's QUDT local name (the unit a value is normalised *to*).
    pub fn canonical_unit(self) -> &'static str {
        match self {
            QuantityKind::Length => "M",
            QuantityKind::Mass => "KiloGM",
            QuantityKind::Time => "SEC",
            QuantityKind::Temperature => "K",
            QuantityKind::Angle => "RAD",
            QuantityKind::DataSize => "BYTE",
        }
    }
}

/// A normalised magnitude: the value expressed in the canonical SI base unit of its
/// [`QuantityKind`], plus that kind (the routing slot). Produced by [`normalise`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Normalised {
    /// The value in the canonical SI base unit (`canonical = lexical * factor + offset`).
    pub canonical_value: f64,
    /// The physical quantity kind — the routing slot. Two `Normalised` values may share a numeric
    /// block only when this matches.
    pub kind: QuantityKind,
}

/// One unit's affine conversion **to** its quantity kind's canonical SI base unit:
/// `canonical = lexical * factor + offset`. For a pure scaling unit (km → m) `offset` is `0.0`;
/// affine units (°C → K) carry a non-zero `offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct UnitConv {
    kind: QuantityKind,
    factor: f64,
    offset: f64,
}

/// Resolve a unit IRI (or bare QUDT local name) to its conversion, or `None` if the unit is not in
/// the bundled table. Accepts a full `http://qudt.org/vocab/unit/<Local>` IRI or just `<Local>`
/// (so a caller that has already stripped the namespace, or uses a different serialisation, still
/// resolves). The match is on the **local name** exactly as QUDT spells it (case-sensitive).
fn lookup(unit_iri: &str) -> Option<UnitConv> {
    let local = unit_iri.strip_prefix(QUDT_UNIT_NS).unwrap_or(unit_iri);
    // The bundled table: (local-name, kind, factor-to-canonical, offset-to-canonical).
    // Curated, deliberately small subset of QUDT. Extending it is purely additive.
    let conv = |kind, factor, offset| UnitConv { kind, factor, offset };
    use QuantityKind::*;
    Some(match local {
        // ---- length (canonical: metre) ----
        "M" | "Meter" | "Metre" => conv(Length, 1.0, 0.0),
        "KiloM" => conv(Length, 1_000.0, 0.0),
        "CentiM" => conv(Length, 0.01, 0.0),
        "MilliM" => conv(Length, 0.001, 0.0),
        "MicroM" => conv(Length, 1e-6, 0.0),
        "NanoM" => conv(Length, 1e-9, 0.0),
        "MI" | "MI_International" => conv(Length, 1_609.344, 0.0), // statute mile
        "YD" => conv(Length, 0.9144, 0.0),                        // yard
        "FT" => conv(Length, 0.3048, 0.0),                        // foot
        "IN" => conv(Length, 0.0254, 0.0),                        // inch
        // ---- mass (canonical: kilogram) ----
        "KiloGM" => conv(Mass, 1.0, 0.0),
        "GM" => conv(Mass, 0.001, 0.0),
        "MilliGM" => conv(Mass, 1e-6, 0.0),
        "MicroGM" => conv(Mass, 1e-9, 0.0),
        "TONNE" | "TON_Metric" => conv(Mass, 1_000.0, 0.0),
        "LB" => conv(Mass, 0.453_592_37, 0.0), // avoirdupois pound
        "OZ" => conv(Mass, 0.028_349_523_125, 0.0),
        // ---- time (canonical: second) ----
        "SEC" => conv(Time, 1.0, 0.0),
        "MilliSEC" => conv(Time, 0.001, 0.0),
        "MicroSEC" => conv(Time, 1e-6, 0.0),
        "MIN" => conv(Time, 60.0, 0.0),
        "HR" => conv(Time, 3_600.0, 0.0),
        "DAY" => conv(Time, 86_400.0, 0.0),
        "WK" => conv(Time, 604_800.0, 0.0),
        "YR" | "YR_Common" => conv(Time, 31_536_000.0, 0.0), // 365-day common year
        // ---- temperature (canonical: kelvin; AFFINE for °C/°F) ----
        "K" => conv(Temperature, 1.0, 0.0),
        "DEG_C" => conv(Temperature, 1.0, 273.15),
        "DEG_F" => conv(Temperature, 5.0 / 9.0, 459.67 * 5.0 / 9.0),
        // ---- plane angle (canonical: radian) ----
        "RAD" => conv(Angle, 1.0, 0.0),
        "DEG" => conv(Angle, core::f64::consts::PI / 180.0, 0.0),
        "GRAD" => conv(Angle, core::f64::consts::PI / 200.0, 0.0),
        // ---- data size (canonical: byte; decimal SI prefixes) ----
        "BYTE" => conv(DataSize, 1.0, 0.0),
        "KiloBYTE" => conv(DataSize, 1_000.0, 0.0),
        "MegaBYTE" => conv(DataSize, 1_000_000.0, 0.0),
        "GigaBYTE" => conv(DataSize, 1_000_000_000.0, 0.0),
        "KibiBYTE" => conv(DataSize, 1_024.0, 0.0),
        "MebiBYTE" => conv(DataSize, 1_048_576.0, 0.0),
        "GibiBYTE" => conv(DataSize, 1_073_741_824.0, 0.0),
        _ => return None,
    })
}

/// Is `unit_iri` a unit the bundled table knows? (`true` ⇒ [`normalise`] succeeds for any finite
/// value.)
pub fn is_known(unit_iri: &str) -> bool {
    lookup(unit_iri).is_some()
}

/// The [`QuantityKind`] a known unit measures, or `None` for an unknown unit. The routing slot
/// without converting a value (e.g. to check two units are comparable before encoding).
pub fn quantity_kind(unit_iri: &str) -> Option<QuantityKind> {
    lookup(unit_iri).map(|c| c.kind)
}

/// Do two unit IRIs measure the **same** physical quantity kind (so their normalised values may
/// share a numeric block)? `false` if either is unknown or they differ — the caller treats a
/// `false` as the **unit-mismatch error** design §2 calls "SHACL-detectable".
pub fn same_quantity(a: &str, b: &str) -> bool {
    match (quantity_kind(a), quantity_kind(b)) {
        (Some(ka), Some(kb)) => ka == kb,
        _ => false,
    }
}

/// Normalise a `(value, unit_iri)` pair to its canonical SI value + quantity kind — the
/// **UNIT-NORMALISE-BEFORE-MAGNITUDE** step (design §2). Returns `None` (fail-closed) when:
///
/// - the unit is **not** in the bundled table (the magnitude is then skipped, never silently
///   treated as dimensionless), or
/// - the lexical `value` is non-finite (`NaN`/`±inf` — a magnitude lane has no order for them), or
/// - the normalised result is non-finite (an overflow under a huge factor).
///
/// On `Some`, feed [`Normalised::canonical_value`] to [`crate::encode::NumericEncoder`] **fitted
/// over canonical values of the same [`Normalised::kind`]**, so `1000 m` and `1 km` land at the
/// identical code and order-preservation holds over *physical* magnitude.
pub fn normalise(value: f64, unit_iri: &str) -> Option<Normalised> {
    if !value.is_finite() {
        return None;
    }
    let conv = lookup(unit_iri)?;
    let canonical = value * conv.factor + conv.offset;
    canonical
        .is_finite()
        .then_some(Normalised { canonical_value: canonical, kind: conv.kind })
}

/// Parse a numeric `lexical` (via [`crate::encode::numeric_value`]) **and** normalise it by
/// `unit_iri` in one call — the convenience entry point from a unit-annotated RDF literal's
/// `(lexical, unit)`. `None` if the lexical is not a finite number or the unit is unknown.
pub fn normalise_lexical(lexical: &str, unit_iri: &str) -> Option<Normalised> {
    let v = crate::encode::numeric_value(lexical)?;
    normalise(v, unit_iri)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- the load-bearing property: same physical quantity → same canonical value ----

    #[test]
    fn same_quantity_normalises_equal() {
        // 1000 m and 1 km are the SAME length → identical canonical value (the design's example).
        let m = normalise(1000.0, "http://qudt.org/vocab/unit/M").unwrap();
        let km = normalise(1.0, "http://qudt.org/vocab/unit/KiloM").unwrap();
        assert_eq!(m.kind, QuantityKind::Length);
        assert_eq!(km.kind, QuantityKind::Length);
        assert!((m.canonical_value - km.canonical_value).abs() < 1e-9, "1000 m == 1 km");
        assert_eq!(m.canonical_value, 1000.0);

        // 1 cm == 10 mm; 2.54 cm == 1 inch (within f64 rounding).
        let cm = normalise(1.0, "CentiM").unwrap();
        let mm = normalise(10.0, "MilliM").unwrap();
        assert!((cm.canonical_value - mm.canonical_value).abs() < 1e-12);
        let inch = normalise(1.0, "IN").unwrap();
        let cm254 = normalise(2.54, "CentiM").unwrap();
        assert!((inch.canonical_value - cm254.canonical_value).abs() < 1e-12, "1 in == 2.54 cm");

        // 1 kg == 1000 g; 1 hour == 3600 s.
        let kg = normalise(1.0, "KiloGM").unwrap();
        let g = normalise(1000.0, "GM").unwrap();
        assert!((kg.canonical_value - g.canonical_value).abs() < 1e-12);
        let hr = normalise(1.0, "HR").unwrap();
        let s = normalise(3600.0, "SEC").unwrap();
        assert_eq!(hr.canonical_value, s.canonical_value);
    }

    #[test]
    fn affine_temperature_units_normalise() {
        // 0 °C == 273.15 K; 100 °C == 373.15 K (affine, offset 273.15).
        let c0 = normalise(0.0, "DEG_C").unwrap();
        assert_eq!(c0.kind, QuantityKind::Temperature);
        assert!((c0.canonical_value - 273.15).abs() < 1e-9);
        let c100 = normalise(100.0, "DEG_C").unwrap();
        assert!((c100.canonical_value - 373.15).abs() < 1e-9);
        // 32 °F == 273.15 K == 0 °C (the freezing point) — affine slope 5/9.
        let f32deg = normalise(32.0, "DEG_F").unwrap();
        assert!((f32deg.canonical_value - 273.15).abs() < 1e-6, "32 F == 273.15 K: {f32deg:?}");
        // 212 °F == 373.15 K == 100 °C (the boiling point).
        let f212 = normalise(212.0, "DEG_F").unwrap();
        assert!((f212.canonical_value - 373.15).abs() < 1e-6, "212 F == 373.15 K: {f212:?}");
    }

    #[test]
    fn angle_and_datasize_units() {
        // 180 degrees == pi radians.
        let deg180 = normalise(180.0, "DEG").unwrap();
        assert!((deg180.canonical_value - core::f64::consts::PI).abs() < 1e-12);
        // 1 KiB == 1024 B; 1 KB (decimal) == 1000 B (SI vs binary prefix kept distinct).
        let kib = normalise(1.0, "KibiBYTE").unwrap();
        assert_eq!(kib.canonical_value, 1024.0);
        let kb = normalise(1.0, "KiloBYTE").unwrap();
        assert_eq!(kb.canonical_value, 1000.0);
        assert_ne!(kib.canonical_value, kb.canonical_value, "KiB != KB by design");
    }

    // ---- fail-closed: unknown unit, non-finite, and the SHACL-detectable mismatch ----

    #[test]
    fn unknown_unit_fails_closed() {
        assert!(normalise(1.0, "http://qudt.org/vocab/unit/NotARealUnit").is_none());
        assert!(normalise(1.0, "http://example.org/myUnit").is_none());
        assert!(!is_known("FOO"));
        assert!(quantity_kind("FOO").is_none());
        // A magnitude with an unknown unit is SKIPPED, never silently dimensionless.
        assert!(normalise_lexical("42", "FOO").is_none());
    }

    #[test]
    fn nonfinite_value_rejected() {
        assert!(normalise(f64::NAN, "M").is_none());
        assert!(normalise(f64::INFINITY, "M").is_none());
        assert!(normalise_lexical("inf", "M").is_none());
        assert!(normalise_lexical("not-a-number", "M").is_none());
    }

    #[test]
    fn unit_mismatch_is_detectable() {
        // A length and a mass are NOT the same quantity — the caller treats this as the
        // unit-mismatch error design §2 calls "SHACL-detectable" (a length is never compared to a
        // mass in one numeric block).
        assert!(!same_quantity("M", "KiloGM"), "length vs mass must be a detectable mismatch");
        assert!(!same_quantity("M", "SEC"), "length vs time must be a detectable mismatch");
        // Same kind, different scale → comparable (the legitimate case).
        assert!(same_quantity("M", "KiloM"));
        assert!(same_quantity("KiloGM", "GM"));
        // Unknown unit on either side → mismatch (fail-closed).
        assert!(!same_quantity("M", "FOO"));
    }

    #[test]
    fn full_iri_and_bare_local_both_resolve() {
        let full = normalise(1.0, "http://qudt.org/vocab/unit/KiloM").unwrap();
        let bare = normalise(1.0, "KiloM").unwrap();
        assert_eq!(full, bare, "full IRI and bare local name must resolve identically");
    }

    #[test]
    fn canonical_unit_is_self_normalising() {
        // Normalising a value already in the canonical unit is the identity (factor 1, offset 0).
        for kind in [
            QuantityKind::Length,
            QuantityKind::Mass,
            QuantityKind::Time,
            QuantityKind::Angle,
            QuantityKind::DataSize,
        ] {
            let n = normalise(7.0, kind.canonical_unit()).unwrap();
            assert_eq!(n.kind, kind);
            assert_eq!(n.canonical_value, 7.0, "canonical unit of {kind:?} must be identity");
        }
    }
}
