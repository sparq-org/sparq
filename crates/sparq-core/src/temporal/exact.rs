//! [GPT-6] Borrowed temporal keys that preserve every fractional digit.

use super::{TemporalKind, Timeline, XSD_DATE, XSD_DATE_TIME, XSD_DATE_TIME_STAMP};
use std::cmp::Ordering;

/// Checks a temporal lexical year's explicit evaluation-capacity range.
///
/// Foreign datatypes and malformed year grammar do not exceed this capacity;
/// their ordinary datatype errors remain the evaluator's responsibility. Valid
/// year grammar exceeding integer storage or the configured range returns false.
/// Parsing uses linear time and no allocation, including extremely wide years.
pub fn year_within_capacity(value: &str, datatype: &str, min: i64, max: i64) -> bool {
    if !matches!(datatype, XSD_DATE | XSD_DATE_TIME | XSD_DATE_TIME_STAMP) {
        return true;
    }
    if min > max {
        return false;
    }
    let value = value.trim_matches([' ', '\t', '\r', '\n']);
    let negative = value.starts_with('-');
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let Some((year, _)) = unsigned.split_once('-') else {
        return true;
    };
    if year.len() < 4
        || (year.len() > 4 && year.starts_with('0'))
        || !year.bytes().all(|byte| byte.is_ascii_digit())
    {
        return true;
    }
    let Ok(year) = year.parse::<i64>() else {
        return false;
    };
    // Year zero is rejected as malformed by the existing temporal parser.
    if year == 0 {
        return true;
    }
    let year = if negative { -year } else { year };
    (min..=max).contains(&year)
}

/// An exact temporal comparison key borrowing its fractional digits.
///
/// Parsing shares the checked calendar and whole-second range of [`Timeline`].
/// Fractional seconds retain arbitrary lexical precision; parsing and comparison
/// take linear time in the input length and allocate no memory.
#[derive(Clone, Copy, Debug)]
pub struct ExactTimeline<'a> {
    seconds: i128,
    fraction: &'a str,
    has_timezone: bool,
}

impl<'a> ExactTimeline<'a> {
    /// Parses a validated dateTime without rounding its comparison key.
    ///
    /// Returns `None` for malformed input or an unsupported calendar magnitude.
    pub fn parse_datetime(value: &'a str) -> Option<Self> {
        // The legacy parser supplies exact checked *whole* seconds and timezone.
        // Its floating fraction is intentionally never used by this key.
        let timeline = Timeline::parse_datetime(value)?;
        let value = value.trim_matches([' ', '\t', '\r', '\n']);
        let (_, time) = value.split_once('T')?;
        let time = time.find(['Z', '+', '-']).map_or(time, |end| &time[..end]);
        let fraction = time.split_once('.').map_or("", |(_, digits)| digits);
        Some(Self {
            seconds: i128::from(timeline.secs) - i128::from(timeline.tz.unwrap_or(0)),
            fraction: fraction.trim_end_matches('0'),
            has_timezone: timeline.tz.is_some(),
        })
    }

    /// Parses a validated date at midnight.
    ///
    /// Returns `None` for malformed input or an unsupported calendar magnitude.
    pub fn parse_date(value: &'a str) -> Option<Self> {
        let timeline = Timeline::parse_date(value)?;
        Some(Self {
            seconds: i128::from(timeline.secs) - i128::from(timeline.tz.unwrap_or(0)),
            fraction: "",
            has_timezone: timeline.tz.is_some(),
        })
    }

    /// Compares values under the XPath partial temporal order.
    ///
    /// Mixed timezone presence is indeterminate within the inclusive fourteen-hour
    /// uncertainty interval. Integer widening makes its endpoint arithmetic exact.
    pub fn compare(self, other: Self) -> Option<Ordering> {
        if self.has_timezone == other.has_timezone {
            return Some(self.compare_instant(other));
        }
        const TIMEZONE_UNCERTAINTY: i128 = 14 * 60 * 60;
        let upper = Self {
            seconds: other.seconds + TIMEZONE_UNCERTAINTY,
            ..other
        };
        let lower = Self {
            seconds: other.seconds - TIMEZONE_UNCERTAINTY,
            ..other
        };
        if self.compare_instant(upper) == Ordering::Greater {
            Some(Ordering::Greater)
        } else if self.compare_instant(lower) == Ordering::Less {
            Some(Ordering::Less)
        } else {
            None
        }
    }

    /// Extends the partial order with exact instants, then timezone presence.
    ///
    /// This deterministic total order positions otherwise indeterminate values;
    /// it is an implementation choice for ORDER BY, never relational equality.
    pub fn compare_total(self, other: Self) -> Ordering {
        self.compare_instant(other)
            .then(self.has_timezone.cmp(&other.has_timezone))
    }

    fn compare_instant(self, other: Self) -> Ordering {
        // Valid digit strings stripped of trailing zeros compare lexically as
        // decimal fractions. A remaining suffix after an equal prefix is positive.
        self.seconds
            .cmp(&other.seconds)
            .then(self.fraction.cmp(other.fraction))
    }
}

/// An exact borrowed temporal key with its datatype family.
#[derive(Clone, Copy, Debug)]
pub struct ExactTemporal<'a> {
    /// The exact value key; lexical source identity remains with the RDF term.
    pub timeline: ExactTimeline<'a>,
    /// The disjoint date or dateTime family.
    pub kind: TemporalKind,
}

impl<'a> ExactTemporal<'a> {
    /// Parses a supported date, dateTime or timezone-bearing dateTimeStamp.
    ///
    /// Returns `None` for foreign datatypes, malformed values or capacity failure.
    pub fn of_lit(value: &'a str, datatype: &str) -> Option<Self> {
        let (kind, timeline) = match datatype {
            XSD_DATE_TIME | XSD_DATE_TIME_STAMP => {
                let timeline = ExactTimeline::parse_datetime(value)?;
                if datatype == XSD_DATE_TIME_STAMP && !timeline.has_timezone {
                    return None;
                }
                (TemporalKind::DateTime, timeline)
            }
            XSD_DATE => (TemporalKind::Date, ExactTimeline::parse_date(value)?),
            _ => return None,
        };
        Some(Self { kind, timeline })
    }

    /// Compares same-family values, preserving indeterminate timezone results.
    pub fn compare(self, other: Self) -> Option<Ordering> {
        (self.kind == other.kind)
            .then(|| self.timeline.compare(other.timeline))
            .flatten()
    }

    /// Orders families first, then exact instants and timezone presence.
    pub fn compare_total(self, other: Self) -> Ordering {
        match (self.kind, other.kind) {
            (TemporalKind::DateTime, TemporalKind::Date) => Ordering::Less,
            (TemporalKind::Date, TemporalKind::DateTime) => Ordering::Greater,
            _ => self.timeline.compare_total(other.timeline),
        }
    }
}
