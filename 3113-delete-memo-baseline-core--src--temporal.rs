//! XSD date/dateTime VALUES and the epoch side-cache cells.
//!
//! [`Timeline`] is the parsed value of an `xsd:date` / `xsd:dateTime` lexical —
//! seconds-from-epoch plus timezone presence — with XPath comparison semantics
//! (both-with-tz / both-without compare directly; MIXED presence is only
//! decidable outside the ±14h window). That XPath order is PARTIAL, which is
//! right for the relational operators (indeterminate = type error) but not for a
//! sort: [`Timeline::cmp_tl_total`] / [`Temporal::cmp_t_total`] are the TOTAL-order
//! extension the `ORDER BY` / `MIN`/`MAX` order uses instead. It lives in core
//! (rather than the engine, which consumes it for FILTER/ORDER BY/`=`) so the
//! graph can precompute a
//! per-term [`Temporal`] cache at load time, exactly like the f64 `numerics`
//! cache: dateTime evaluation then never round-trips the dictionary per row.
//!
//! [`Temporal`] is the cache cell: the literal's datatype family plus the
//! PRECOMPUTED comparison key `Timeline::instant()` (an f64 — bit-identical to
//! what the per-row parse would feed `partial_cmp`, so cached comparisons match
//! the dict-based path exactly, including sub-second precision and the f64
//! collapse of far-apart instants) and the timezone-presence bit that decides
//! the mixed-presence indeterminate window. `xsd:time` is NOT cached: the
//! engine compares it lexically (OtherXsd), not on the timeline, and a cache
//! must not change that.

use std::cmp::Ordering;

/// An xsd:date / xsd:dateTime VALUE: seconds-from-epoch of the local time, fractional
/// seconds, and the timezone offset when present. Comparison follows XSD: both-with-tz
/// and both-without compare directly; MIXED presence is only decidable outside the
/// ±14h window (inside it the comparison is indeterminate — a SPARQL type error).
#[derive(Clone, Copy, Debug)]
pub struct Timeline {
    pub secs: i64,
    pub frac: f64,
    pub tz: Option<i64>,
}

impl Timeline {
    pub fn parse_datetime(s: &str) -> Option<Timeline> {
        let (date, rest) = s.split_once('T')?;
        let (time, tz) = match rest.find(['Z', '+', '-']) {
            Some(i) => (&rest[..i], Some(parse_tz(&rest[i..])?)),
            None => (rest, None),
        };
        let days = parse_civil_date(date)?;
        let mut t = time.split(':');
        let h: i64 = t.next()?.parse().ok()?;
        let mi: i64 = t.next()?.parse().ok()?;
        let sec_lex = t.next()?;
        if t.next().is_some() {
            return None;
        }
        let sec: f64 = sec_lex.parse().ok()?;
        Some(Timeline {
            secs: days * 86_400 + h * 3600 + mi * 60 + sec.trunc() as i64,
            frac: sec.fract(),
            tz,
        })
    }

    pub fn parse_date(s: &str) -> Option<Timeline> {
        // The timezone suffix starts after the day: "...-23Z" / "...-23+05:00". A bare
        // date's own hyphens must not be mistaken for an offset sign, so require the
        // ":" of "±hh:mm" at the right position.
        let (date, tz) = if let Some(d) = s.strip_suffix('Z') {
            (d, Some(0))
        } else if s.len() > 10 && matches!(s.as_bytes()[s.len() - 6], b'+' | b'-') && s.as_bytes()[s.len() - 3] == b':' {
            (&s[..s.len() - 6], Some(parse_tz(&s[s.len() - 6..])?))
        } else {
            (s, None)
        };
        Some(Timeline { secs: parse_civil_date(date)? * 86_400, frac: 0.0, tz })
    }

    /// The absolute instant (treating an absent timezone as UTC) in seconds.
    pub fn instant(&self) -> f64 {
        (self.secs - self.tz.unwrap_or(0)) as f64 + self.frac
    }

    pub fn cmp_tl(a: Timeline, b: Timeline) -> Option<Ordering> {
        cmp_instants(a.instant(), a.tz.is_some(), b.instant(), b.tz.is_some())
    }

    /// The TOTAL-order EXTENSION of [`cmp_tl`](Self::cmp_tl) — for the `ORDER BY` /
    /// `MIN`/`MAX` total order ONLY. Never for the relational operators: `<` / `>` / `=`
    /// keep [`cmp_tl`](Self::cmp_tl)'s indeterminate window as a SPARQL type error.
    ///
    /// Orders by the INSTANT-ASSUMING-UTC ([`instant`](Self::instant)) first, then by
    /// timezone PRESENCE (floating before zoned) as the tiebreak. Where
    /// [`cmp_tl`](Self::cmp_tl) decides, this AGREES with it (it decides by exactly that
    /// instant comparison); the tiebreak only positions the pairs it leaves indeterminate.
    ///
    /// # Why an extension is needed
    ///
    /// [`cmp_tl`](Self::cmp_tl) is a PARTIAL order: a tz-less dateTime against a tz-carrying
    /// one inside the ±14h window is indeterminate. A comparator that falls back to the
    /// LEXICAL form on those pairs mixes timeline-decided and lexical-decided pairs in one
    /// kind, which is intransitive — the shape of the sq-wjl8i witnesses. Concretely, with
    /// a lexical fallback: `"2024-03-15T12:00:00-01:00"` and `"2024-03-15T14:00:00+01:00"`
    /// are the SAME instant (timeline-Equal), yet the floating `"2024-03-15T13:00:00"`
    /// sits lexically BETWEEN them — so `x = y`, `x < z` and `z < y`, and `ORDER BY`'s
    /// `sort_by` is fed an inconsistent comparator.
    ///
    /// # Why the tiebreak is timezone PRESENCE, not the lexical form
    ///
    /// The witness above is not repaired by ordering the indeterminate pairs lexically
    /// *after* the instant — the equal-instant class still contains zoned lexicals on both
    /// lexical sides of a floating one. Presence is a function of the TERM (like the
    /// instant), so the whole within-kind order is the lexicographic key
    /// `(instant, has_tz)`: a genuine total order, whose `Equal` is a real equivalence.
    ///
    /// This position for the indeterminate window is a documented sparq EXTENSION, not a
    /// spec claim: SPARQL 1.1 §15.1 leaves it open, and the relational semantics are
    /// unchanged. [SONNET-4.6] sq-2k5py
    #[inline]
    pub fn cmp_tl_total(a: Timeline, b: Timeline) -> Ordering {
        cmp_instants_total(a.instant(), a.tz.is_some(), b.instant(), b.tz.is_some())
    }
}

/// The XPath dateTime/date comparison on precomputed instants: direct when both
/// or neither operand carries a timezone; with MIXED presence only decidable
/// outside the ±14h window (inside it: indeterminate -> `None`).
#[inline]
fn cmp_instants(ai: f64, a_tz: bool, bi: f64, b_tz: bool) -> Option<Ordering> {
    // Same (or no) timezone: a direct compare. With MIXED presence the order is
    // only decidable outside the ±14h window; inside it the result is indeterminate.
    if a_tz == b_tz || (ai - bi).abs() > 14.0 * 3600.0 {
        ai.partial_cmp(&bi)
    } else {
        None
    }
}

/// The TOTAL-order extension of `cmp_instants`: the XPath partial order where it
/// decides, else the instant order with timezone PRESENCE (floating < zoned) as the
/// tiebreak. See [`Timeline::cmp_tl_total`] for the rationale and the intransitivity
/// witness this closes. [SONNET-4.6] sq-2k5py
#[inline]
fn cmp_instants_total(ai: f64, a_tz: bool, bi: f64, b_tz: bool) -> Ordering {
    match cmp_instants(ai, a_tz, bi, b_tz) {
        Some(o) => o,
        // The indeterminate mixed-presence window (and the NaN instant an ill-formed-but
        // -parsing lexical can produce): order by instant, then by presence. `partial_cmp`
        // first keeps `-0.0 == 0.0` — the verdict `cmp_instants` itself would give, so the
        // extension can never contradict the partial order it extends — with `total_cmp`
        // only for a NaN instant, which `partial_cmp` cannot place at all.
        None => ai.partial_cmp(&bi).unwrap_or_else(|| ai.total_cmp(&bi)).then(a_tz.cmp(&b_tz)),
    }
}

/// `"Z"` / `"±hh:mm"` -> offset seconds.
pub fn parse_tz(tz: &str) -> Option<i64> {
    if tz == "Z" {
        return Some(0);
    }
    let (sign, hm) = tz.split_at(1);
    let (h, m) = hm.split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    let off = h * 3600 + m * 60;
    Some(if sign == "-" { -off } else { off })
}

/// `[-]YYYY-MM-DD` -> days from the epoch (Howard Hinnant's days_from_civil).
pub fn parse_civil_date(date: &str) -> Option<i64> {
    let neg = date.starts_with('-');
    let mut p = date.strip_prefix('-').unwrap_or(date).split('-');
    let y: i64 = p.next()?.parse().ok()?;
    let y = if neg { -y } else { y };
    let m: i64 = p.next()?.parse().ok()?;
    let d: i64 = p.next()?.parse().ok()?;
    if p.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// The datatype family of a cached temporal value. `xsd:dateTime` and
/// `xsd:dateTimeStamp` share a family (they compare on the timeline); `xsd:date`
/// is a DISJOINT family (cross-family `=` is false, ordering a type error).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalKind {
    DateTime,
    Date,
}

/// A cached temporal VALUE: the precomputed comparison key of one dictionary term.
/// `instant` is exactly [`Timeline::instant`] of the parsed lexical, so comparisons
/// through the cache are bit-identical to re-parsing the term per row.
#[derive(Clone, Copy, Debug)]
pub struct Temporal {
    pub instant: f64,
    pub has_tz: bool,
    pub kind: TemporalKind,
}

/// XSD namespace datatype IRIs (kept as plain strs: this module must not depend
/// on oxrdf — the dict hands the cache builder borrowed datatype strings).
const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const XSD_DATE_TIME_STAMP: &str = "http://www.w3.org/2001/XMLSchema#dateTimeStamp";
const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";

impl Temporal {
    /// The cache cell for a literal's (lexical, datatype) — `None` when the datatype
    /// is not dateTime/dateTimeStamp/date or the lexical is ill-formed (ill-formed
    /// temporals stay on the slow path, which yields the type-error semantics).
    pub fn of_lit(value: &str, datatype: &str) -> Option<Temporal> {
        let (kind, tl) = match datatype {
            XSD_DATE_TIME | XSD_DATE_TIME_STAMP => (TemporalKind::DateTime, Timeline::parse_datetime(value)?),
            XSD_DATE => (TemporalKind::Date, Timeline::parse_date(value)?),
            _ => return None,
        };
        Some(Temporal { instant: tl.instant(), has_tz: tl.tz.is_some(), kind })
    }

    /// XPath comparison of two cached temporals: `None` for cross-family operands
    /// (dateTime vs date) and for the mixed-timezone-presence indeterminate window —
    /// exactly the cases the engine's per-row parse path treats as type errors.
    #[inline]
    pub fn cmp_t(a: Temporal, b: Temporal) -> Option<Ordering> {
        if a.kind != b.kind {
            return None;
        }
        cmp_instants(a.instant, a.has_tz, b.instant, b.has_tz)
    }

    /// The TOTAL order over cached temporals, for the `ORDER BY` / `MIN`/`MAX` total order
    /// ONLY (relational `<` / `>` / `=` keep [`cmp_t`](Self::cmp_t)'s type errors): the
    /// cross-family pair ranks KIND-FIRST — dateTime before date, the literal-kind rank the
    /// engine and the shared comparator apply — and a same-family pair by
    /// [`Timeline::cmp_tl_total`]'s instant-then-timezone-presence order.
    ///
    /// This is the ONE definition the three lock-step call sites (the ORDER BY sort-cell
    /// comparator, the id-level MIN/MAX fold, and the `CompareTerm::strict_cmp` seam the
    /// substrate's total order drives) share, so they cannot drift apart again.
    /// [SONNET-4.6] sq-2k5py
    #[inline]
    pub fn cmp_t_total(a: Temporal, b: Temporal) -> Ordering {
        match (a.kind, b.kind) {
            (TemporalKind::DateTime, TemporalKind::Date) => Ordering::Less,
            (TemporalKind::Date, TemporalKind::DateTime) => Ordering::Greater,
            _ => cmp_instants_total(a.instant, a.has_tz, b.instant, b.has_tz),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    fn dt(s: &str) -> Temporal {
        Temporal::of_lit(s, XSD_DATE_TIME).expect("valid dateTime")
    }

    #[test]
    fn equal_instants_across_timezones() {
        // XPath: equal instants compare equal even with different offsets.
        let a = dt("2024-03-15T13:00:00Z");
        let b = dt("2024-03-15T14:00:00+01:00");
        assert_eq!(Temporal::cmp_t(a, b), Some(Equal));
        let c = dt("2024-03-15T08:00:00-05:00");
        assert_eq!(Temporal::cmp_t(a, c), Some(Equal));
    }

    #[test]
    fn mixed_presence_window_is_indeterminate() {
        let zoned = dt("2024-03-15T13:00:00Z");
        let floating = dt("2024-03-15T13:00:00");
        // Same wall time, one zoned one floating: inside ±14h -> indeterminate.
        assert_eq!(Temporal::cmp_t(zoned, floating), None);
        // Outside the window the order is decidable.
        let far = dt("2024-03-17T13:00:00");
        assert_eq!(Temporal::cmp_t(zoned, far), Some(Less));
    }

    #[test]
    fn subsecond_precision_preserved() {
        let a = dt("2024-03-15T13:00:00.250Z");
        let b = dt("2024-03-15T13:00:00.500Z");
        assert_eq!(Temporal::cmp_t(a, b), Some(Less));
        assert_eq!(Temporal::cmp_t(b, a), Some(Greater));
        let c = dt("2024-03-15T13:00:00.250Z");
        assert_eq!(Temporal::cmp_t(a, c), Some(Equal));
    }

    #[test]
    fn date_and_datetime_are_disjoint_families() {
        let d = Temporal::of_lit("2024-03-15", XSD_DATE).unwrap();
        let t = dt("2024-03-15T00:00:00Z");
        assert_eq!(Temporal::cmp_t(d, t), None);
    }

    #[test]
    fn cache_cell_matches_per_row_parse() {
        // The cell must carry EXACTLY the parsed Timeline's instant/tz-presence.
        for lex in ["2024-03-15T13:45:30.123456Z", "2024-03-15T13:45:30-09:30", "-0044-03-15T12:00:00", "9999-12-31T23:59:59.999Z"] {
            let tl = Timeline::parse_datetime(lex).unwrap();
            let t = dt(lex);
            assert_eq!(t.instant.to_bits(), tl.instant().to_bits(), "instant differs for {lex}");
            assert_eq!(t.has_tz, tl.tz.is_some());
        }
    }

    #[test]
    fn ill_formed_and_foreign_datatypes_are_not_cached() {
        assert!(Temporal::of_lit("not-a-date", XSD_DATE_TIME).is_none());
        assert!(Temporal::of_lit("2024-13-99T99:99:99", XSD_DATE_TIME).is_none());
        // xsd:time compares lexically in the engine — must NOT enter the cache.
        assert!(Temporal::of_lit("13:00:00", "http://www.w3.org/2001/XMLSchema#time").is_none());
        assert!(Temporal::of_lit("42", "http://www.w3.org/2001/XMLSchema#integer").is_none());
    }

    // [SONNET-4.6] sq-2k5py — the TOTAL-order extension over the indeterminate window: a
    // direct test per new public fn (`Timeline::cmp_tl_total` / `Temporal::cmp_t_total`)
    // plus the intransitivity WITNESS the extension exists to close.

    /// `Timeline::cmp_tl_total` must AGREE with `cmp_tl` on every pair `cmp_tl` decides,
    /// and place the indeterminate ones by instant-then-timezone-presence.
    #[test]
    fn cmp_tl_total_extends_cmp_tl_without_contradicting_it() {
        let p = |s: &str| Timeline::parse_datetime(s).unwrap();
        // Agreement: wherever the XPath partial order decides, the extension repeats it.
        for (a, b) in [
            ("2024-03-15T13:00:00Z", "2024-03-15T14:00:00+01:00"), // equal instants, both zoned
            ("2024-03-15T12:00:00", "2024-03-15T13:00:00"),        // both floating
            ("2024-03-15T13:00:00Z", "2024-03-17T13:00:00"),       // mixed, OUTSIDE the window
            ("2024-03-15T13:00:00.250Z", "2024-03-15T13:00:00.500Z"), // sub-second
        ] {
            let decided = Timeline::cmp_tl(p(a), p(b)).expect("pair is XPath-decidable");
            assert_eq!(Timeline::cmp_tl_total(p(a), p(b)), decided, "extension contradicts cmp_tl on {a} vs {b}");
        }
        // The indeterminate window is now DECIDED: same instant, so timezone presence
        // breaks the tie — the floating value sorts before the zoned one.
        let (zoned, floating) = (p("2024-03-15T13:00:00Z"), p("2024-03-15T13:00:00"));
        assert_eq!(Timeline::cmp_tl(zoned, floating), None, "still a relational type error");
        assert_eq!(Timeline::cmp_tl_total(floating, zoned), Less);
        assert_eq!(Timeline::cmp_tl_total(zoned, floating), Greater);
        // Inside the window but NOT the same instant: the instant still decides first.
        assert_eq!(Timeline::cmp_tl_total(floating, p("2024-03-15T13:00:00.250Z")), Less);
        // Reflexive, and equal-instant same-presence pairs stay Equal (no spurious order).
        assert_eq!(Timeline::cmp_tl_total(zoned, zoned), Equal);
        assert_eq!(Timeline::cmp_tl_total(zoned, p("2024-03-15T08:00:00-05:00")), Equal);
    }

    /// The WITNESS sq-2k5py closes: two zoned dateTimes are the same instant (so
    /// timeline-Equal) while a floating one sits lexically BETWEEN them — under a lexical
    /// fallback that is `x = y`, `x < z`, `z < y`, an inconsistent comparator. The
    /// extension orders all three by (instant, presence), so `z` sits strictly below the
    /// equal pair and transitivity holds.
    #[test]
    fn witness_indeterminate_window_lexical_fallback_was_intransitive() {
        let p = |s: &str| Timeline::parse_datetime(s).unwrap();
        let (x, y, z) = ("2024-03-15T12:00:00-01:00", "2024-03-15T14:00:00+01:00", "2024-03-15T13:00:00");
        // x and y are the SAME instant; both are indeterminate against the floating z.
        assert_eq!(Timeline::cmp_tl(p(x), p(y)), Some(Equal));
        assert_eq!(Timeline::cmp_tl(p(x), p(z)), None);
        assert_eq!(Timeline::cmp_tl(p(y), p(z)), None);
        // The OLD lexical fallback put z strictly between the equal pair — intransitive.
        assert_eq!(x.cmp(z), Less, "the lexical fallback's x < z leg");
        assert_eq!(y.cmp(z), Greater, "the lexical fallback's y > z leg");
        // The extension: z (floating) is below BOTH members of the equal instant class.
        assert_eq!(Timeline::cmp_tl_total(p(x), p(y)), Equal);
        assert_eq!(Timeline::cmp_tl_total(p(z), p(x)), Less);
        assert_eq!(Timeline::cmp_tl_total(p(z), p(y)), Less);
    }

    /// `Temporal::cmp_t_total` — the cached twin: KIND-FIRST (dateTime before date) and
    /// then the same instant-then-presence order, agreeing with `cmp_t` where it decides.
    #[test]
    fn cmp_t_total_is_kind_first_and_total() {
        let d = |s: &str| Temporal::of_lit(s, XSD_DATE).expect("valid date");
        // Cross-family: kind rank decides, never a value or lexical coercion.
        assert_eq!(Temporal::cmp_t(dt("2024-03-15T00:00:00Z"), d("1999-01-01")), None);
        assert_eq!(Temporal::cmp_t_total(dt("2024-03-15T00:00:00Z"), d("1999-01-01")), Less);
        assert_eq!(Temporal::cmp_t_total(d("1999-01-01"), dt("2024-03-15T00:00:00Z")), Greater);
        // Same family: agrees with `cmp_t` where it decides…
        assert_eq!(Temporal::cmp_t_total(dt("2024-03-15T13:00:00Z"), dt("2024-03-15T14:00:00+01:00")), Equal);
        assert_eq!(Temporal::cmp_t_total(dt("2024-03-15T13:00:00Z"), dt("2024-03-16T13:00:00Z")), Less);
        // …and decides the indeterminate window by timezone presence.
        assert_eq!(Temporal::cmp_t(dt("2024-03-15T13:00:00Z"), dt("2024-03-15T13:00:00")), None);
        assert_eq!(Temporal::cmp_t_total(dt("2024-03-15T13:00:00"), dt("2024-03-15T13:00:00Z")), Less);
        // The date family gets the same extension (a tz-less date vs a zoned one).
        assert_eq!(Temporal::cmp_t(d("2024-03-15"), d("2024-03-15Z")), None);
        assert_eq!(Temporal::cmp_t_total(d("2024-03-15"), d("2024-03-15Z")), Less);
    }

    // [OPUS-4.8] sq-bif — the tests below close real gaps in this module's default surface:
    // `Timeline::cmp_tl` (the direct timeline-comparison API — previously only `Temporal::cmp_t`
    // was exercised), `parse_date`'s timezone-OFFSET branch (only the `Z` branch was covered),
    // the `parse_tz` / `parse_civil_date` parsers' boundary + rejection behaviour, the
    // dateTime/dateTimeStamp shared family, and `instant()`'s timezone normalisation.

    /// `Timeline::cmp_tl` is the public timeline comparison the engine calls before a cache
    /// cell exists; it must follow the SAME XPath rules as the cached `Temporal::cmp_t` —
    /// both-zoned/both-floating compare directly, mixed presence is indeterminate inside the
    /// ±14h window and decidable outside it. Previously untested.
    #[test]
    fn cmp_tl_matches_xpath_and_temporal_cmp() {
        let p = |s: &str| Timeline::parse_datetime(s).unwrap();
        // Equal instants across timezones compare Equal.
        assert_eq!(
            Timeline::cmp_tl(p("2024-03-15T13:00:00Z"), p("2024-03-15T14:00:00+01:00")),
            Some(Equal),
        );
        // Both floating (no tz): a direct compare.
        assert_eq!(Timeline::cmp_tl(p("2024-03-15T12:00:00"), p("2024-03-15T13:00:00")), Some(Less));
        // Mixed presence inside the ±14h window: indeterminate (a SPARQL type error).
        assert_eq!(Timeline::cmp_tl(p("2024-03-15T13:00:00Z"), p("2024-03-15T13:00:00")), None);
        // Mixed presence OUTSIDE the window: decidable.
        assert_eq!(Timeline::cmp_tl(p("2024-03-15T13:00:00Z"), p("2024-03-17T13:00:00")), Some(Less));

        // And `cmp_tl` agrees with the cached `Temporal::cmp_t` on the same lexicals.
        for (a, b) in [
            ("2024-03-15T13:00:00Z", "2024-03-15T14:00:00+01:00"),
            ("2024-03-15T13:00:00Z", "2024-03-15T13:00:00"),
            ("2024-03-15T13:00:00Z", "2024-03-17T13:00:00"),
            ("2024-03-15T12:00:00", "2024-03-15T13:00:00"),
        ] {
            assert_eq!(Timeline::cmp_tl(p(a), p(b)), Temporal::cmp_t(dt(a), dt(b)), "cmp_tl/cmp_t disagree on {a} vs {b}");
        }
    }

    /// `parse_date` must accept a timezone OFFSET suffix (`+hh:mm` / `-hh:mm`), not only `Z`,
    /// and must NOT mistake a bare date's own hyphens for an offset sign. The offset branch
    /// (the `len > 10 && sign && ':'` path) was previously uncovered.
    #[test]
    fn parse_date_handles_timezone_offset_and_bare_date() {
        // Bare date: no timezone, midnight UTC, day 0 == 1970-01-01.
        let epoch = Timeline::parse_date("1970-01-01").unwrap();
        assert_eq!(epoch.secs, 0);
        assert!(epoch.tz.is_none());

        // `Z` suffix: tz present and 0.
        let z = Timeline::parse_date("2024-03-15Z").unwrap();
        assert_eq!(z.tz, Some(0));

        // Positive and negative OFFSET suffixes are parsed (the previously-untested branch).
        let plus = Timeline::parse_date("2024-03-15+05:00").unwrap();
        assert_eq!(plus.tz, Some(5 * 3600));
        let minus = Timeline::parse_date("2024-03-15-09:30").unwrap();
        assert_eq!(minus.tz, Some(-(9 * 3600 + 30 * 60)));
        // The civil date is the same regardless of the offset suffix.
        assert_eq!(plus.secs, minus.secs);
        assert_eq!(plus.secs, Timeline::parse_date("2024-03-15").unwrap().secs);

        // A NEGATIVE-YEAR bare date must not be read as having a trailing offset (its leading
        // `-` is the year sign, and there is no `:` six chars from the end).
        let bce = Timeline::parse_date("-0044-03-15").unwrap();
        assert!(bce.tz.is_none(), "a BCE year sign must not be parsed as a timezone offset");
        assert!(bce.secs < 0, "44 BCE is before the epoch");
    }

    /// `parse_tz` directly: `Z` is +0, signed `±hh:mm` offsets resolve to seconds, and a
    /// malformed offset is rejected.
    #[test]
    fn parse_tz_parses_offsets_and_rejects_malformed() {
        assert_eq!(parse_tz("Z"), Some(0));
        assert_eq!(parse_tz("+00:00"), Some(0));
        assert_eq!(parse_tz("+05:30"), Some(5 * 3600 + 30 * 60));
        assert_eq!(parse_tz("-08:00"), Some(-8 * 3600));
        // Missing the `:hh` minute field is malformed.
        assert_eq!(parse_tz("+05"), None);
        assert_eq!(parse_tz("garbage"), None);
    }

    /// `parse_civil_date` (Howard Hinnant's days_from_civil): the epoch is day 0, ordering is
    /// monotonic, negative (BCE) years parse, and out-of-range month/day or extra components
    /// are rejected.
    #[test]
    fn parse_civil_date_epoch_ordering_and_validation() {
        assert_eq!(parse_civil_date("1970-01-01"), Some(0), "epoch is day 0");
        assert_eq!(parse_civil_date("1970-01-02"), Some(1));
        assert_eq!(parse_civil_date("1969-12-31"), Some(-1));
        // A whole non-leap year later is +365 days.
        assert_eq!(parse_civil_date("1971-01-01"), Some(365));
        // 2000 is a leap year (divisible by 400): Feb 29 is valid and day-of-year ordered.
        assert!(parse_civil_date("2000-02-29").is_some());
        assert!(parse_civil_date("2000-02-29") < parse_civil_date("2000-03-01"));
        // BCE years parse to large-negative day counts.
        assert!(parse_civil_date("-0001-12-31").unwrap() < 0);

        // Rejections: month/day out of the 1..=12 / 1..=31 ranges, and an extra component.
        assert!(parse_civil_date("2024-13-01").is_none(), "month 13 rejected");
        assert!(parse_civil_date("2024-00-01").is_none(), "month 0 rejected");
        assert!(parse_civil_date("2024-01-32").is_none(), "day 32 rejected");
        assert!(parse_civil_date("2024-01-00").is_none(), "day 0 rejected");
        assert!(parse_civil_date("2024-01-01-01").is_none(), "extra component rejected");
        assert!(parse_civil_date("notanumber").is_none());
    }

    /// `xsd:dateTime` and `xsd:dateTimeStamp` share ONE temporal family (they compare on the
    /// timeline together), while `xsd:date` is disjoint. Only the dateTime path was exercised
    /// before; this pins dateTimeStamp's family membership and cross-family indeterminacy.
    #[test]
    fn datetime_and_datetimestamp_share_a_family_date_is_disjoint() {
        let stamp = Temporal::of_lit("2024-03-15T13:00:00Z", XSD_DATE_TIME_STAMP).unwrap();
        assert_eq!(stamp.kind, TemporalKind::DateTime, "dateTimeStamp caches as the DateTime family");
        let dttime = dt("2024-03-15T13:00:00Z");
        // Same instant, both in the DateTime family: comparable and Equal.
        assert_eq!(Temporal::cmp_t(stamp, dttime), Some(Equal));
        // A date is a disjoint family: comparing across families is a type error (None).
        let date = Temporal::of_lit("2024-03-15", XSD_DATE).unwrap();
        assert_eq!(date.kind, TemporalKind::Date);
        assert_eq!(Temporal::cmp_t(stamp, date), None);
    }

    /// `Timeline::instant()` normalises a zoned time to UTC (subtracting the offset) and treats
    /// an absent timezone as UTC, so two lexicals naming the same absolute instant share one
    /// `instant`, and the fractional second is carried.
    #[test]
    fn instant_normalises_timezone_to_utc() {
        let utc = Timeline::parse_datetime("2024-03-15T13:00:00Z").unwrap();
        let off = Timeline::parse_datetime("2024-03-15T14:00:00+01:00").unwrap();
        // Same absolute instant despite different wall clock + offset.
        assert_eq!(utc.instant().to_bits(), off.instant().to_bits());
        // A floating (no-tz) time is treated as UTC, so it equals the same wall time zoned `Z`.
        let floating = Timeline::parse_datetime("2024-03-15T13:00:00").unwrap();
        assert_eq!(floating.instant(), utc.instant());
        // The fractional second survives into the instant.
        let frac = Timeline::parse_datetime("2024-03-15T13:00:00.25Z").unwrap();
        assert_eq!(frac.instant() - utc.instant(), 0.25);
    }
}
