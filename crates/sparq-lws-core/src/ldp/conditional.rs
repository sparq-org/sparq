// AUTHORED-BY Claude Opus 4.8
//! HTTP conditional-request preconditions (`If-Match` / `If-None-Match`) over the strong ETag.
//!
//! This is pure value logic (no I/O): given a request's precondition headers and the resource's
//! current ETag (or its absence, when the resource does not exist), it decides whether a mutating
//! request may proceed (RFC 9110 §13.1–§13.2). The handler holds the I/O; this module holds the
//! exact comparison rules so they are exhaustively unit-testable.
//!
//! The server emits only **strong** ETags (`"…"`). Validator strength is honoured per RFC 9110:
//! `If-Match` uses **strong comparison** (§13.1.1 — "the server MUST NOT … weak"), so an inbound
//! weak validator (`W/"…"`) can NEVER satisfy `If-Match` and the request fails; `If-None-Match` uses
//! **weak comparison** (§13.1.2), so a `W/`-prefixed validator matches by its opaque tag. The
//! wildcard `*` is handled per spec: `If-None-Match: *` ⇒ "only if it does NOT exist" (the create
//! guard); `If-Match: *` ⇒ "only if it DOES exist".
//!
//! ## Representation-variant tags: `"<state>+<variant>"`
//!
//! An entity-tag identifies a **representation**, not a resource (RFC 9110 §8.8.3). This server
//! stores ONE representation per resource but content-negotiates RDF (Turtle ↔ JSON-LD), so a GET
//! whose negotiated format differs from the stored one is served a DIFFERENT representation — and
//! must carry a different validator, or a client holding the Turtle tag would get a spurious 304
//! for a JSON-LD body it never saw. The handler therefore mints [`variant_etag`]: the stored
//! (state) tag plus a `+<variant>` suffix inside the quotes (`"7-1a2b"` → `"7-1a2b+jsonld"`). The
//! base tags this server mints (`"<len>-<hash>"`) never contain `+`, so the suffix is unambiguous.
//!
//! The two comparison audiences then differ deliberately:
//!
//! - **Read preconditions** ([`evaluate_read`]) compare EXACTLY against the negotiated
//!   representation's tag — a Turtle tag never 304s a JSON-LD response.
//! - **Write preconditions** ([`evaluate`]) compare the **STATE part** (the tag with any
//!   `+<variant>` suffix stripped): every negotiated variant of one stored state is equally
//!   current, so a client may round-trip `GET` (any `Accept`) → `If-Match` PUT/PATCH/DELETE with
//!   the ETag it received. A write conditional guards resource STATE, not the wire format the
//!   client happened to read it in.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ServerError;

/// The outcome of evaluating preconditions: proceed, or fail with the spec-mandated status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precondition {
    /// Preconditions are satisfied — the request may proceed.
    Proceed,
    /// A precondition was not met — the request must be rejected with 412 Precondition Failed.
    ///
    /// (For a GET, an unmet `If-None-Match` is instead a 304; this server applies preconditions only
    /// to mutating verbs (PUT/PATCH/DELETE), where the failure status is always 412 — see RFC 9110
    /// §13.1.2: "for methods other than GET/HEAD … 412".)
    Failed,
}

/// Evaluate the `If-Match` / `If-None-Match` preconditions for a mutating request.
///
/// `if_match` / `if_none_match` are the raw header values (already validated as UTF-8 by the HTTP
/// layer). `current` is `Some(etag)` if the target exists with that strong ETag, or `None` if the
/// target does not currently exist.
///
/// Precedence follows RFC 9110 §13.2.2: when both are present, `If-Match` is evaluated **first**.
/// (`If-None-Match` then still applies — but a request that supplies both a matching `If-Match` and
/// an `If-None-Match` for the same existing tag is contradictory and fails on the `If-None-Match`.)
pub fn evaluate(
    if_match: Option<&str>,
    if_none_match: Option<&str>,
    current: Option<&str>,
) -> Precondition {
    evaluate_write(if_match, if_none_match, None, current, None)
}

/// [`evaluate`] plus `If-Unmodified-Since` (RFC 9110 §13.1.4), in the §13.2.2 order: `If-Match`
/// first; `If-Unmodified-Since` only when `If-Match` is ABSENT; then `If-None-Match`.
///
/// `last_modified` is the target's recorded modification time (`None` when it does not exist or the
/// store records none). `If-Unmodified-Since` fails the request with 412 iff the target's
/// modification time, in whole seconds, is LATER than the header date. It is ignored, as §13.1.4
/// requires or permits, when `If-Match` is present, when the date is not a valid IMF-fixdate, and
/// when there is no modification time to compare (including a target that does not exist).
pub fn evaluate_write(
    if_match: Option<&str>,
    if_none_match: Option<&str>,
    if_unmodified_since: Option<&str>,
    current: Option<&str>,
    last_modified: Option<SystemTime>,
) -> Precondition {
    // --- If-Match: proceed only if the current representation matches one of the listed tags, by
    // STRONG comparison (RFC 9110 §13.1.1) — a weak (`W/`) validator never satisfies If-Match.
    // Writes guard resource STATE, so the comparison is on the state part: a `+<variant>` tag a
    // client received from a content-negotiated GET matches the stored state it derives from (the
    // GET → If-Match round-trip; see the module doc).
    if let Some(im) = if_match {
        let ok = match current {
            // `If-Match: *` ⇒ the resource must exist.
            _ if is_wildcard(im) => current.is_some(),
            Some(cur) => tag_list(im).any(|t| t.matches_strong_state(cur)),
            // No current representation can match a concrete tag list.
            None => false,
        };
        if !ok {
            return Precondition::Failed;
        }
    } else if let (Some(date), Some(lm)) = (
        unmodified_since_applies(if_match, if_unmodified_since),
        last_modified,
    ) {
        // --- If-Unmodified-Since (only without If-Match): the target must not have changed after
        // the client's date.
        if system_time_to_unix(lm).is_some_and(|lm_secs| lm_secs > date) {
            return Precondition::Failed;
        }
    }

    // --- If-None-Match: proceed only if NONE of the listed tags match (the create / no-overwrite
    // guard), by WEAK comparison (RFC 9110 §13.1.2) on the state part (any negotiated variant of
    // the current state is "current" for a write guard). `If-None-Match: *` ⇒ the resource must
    // NOT exist.
    if let Some(inm) = if_none_match {
        let matched = match current {
            _ if is_wildcard(inm) => current.is_some(),
            Some(cur) => tag_list(inm).any(|t| t.matches_weak_state(cur)),
            None => false,
        };
        if matched {
            return Precondition::Failed;
        }
    }

    Precondition::Proceed
}

/// Map a [`Precondition`] outcome to a `Result` the handler can `?`-propagate.
pub fn require(p: Precondition) -> Result<(), ServerError> {
    match p {
        Precondition::Proceed => Ok(()),
        Precondition::Failed => Err(ServerError::PreconditionFailed),
    }
}

/// The outcome of evaluating the READ preconditions of a GET/HEAD (RFC 9110 §13): serve the normal
/// response, or short-circuit **304 Not Modified**.
///
/// A read precondition NEVER fails with 412 — for GET/HEAD an unmet `If-None-Match` / a satisfied
/// `If-Modified-Since` is a 304 (RFC 9110 §13.1.2 / §13.1.3), not a rejection. That is the whole
/// point of the read fast-path: a 304 carries the validators and no body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPrecondition {
    /// No read precondition short-circuited — serve the normal 200 (or 206 / 406 / 416) response.
    Proceed,
    /// A read precondition matched — return **304 Not Modified** (the validators, no body).
    NotModified,
}

/// Evaluate the READ preconditions (`If-None-Match`, then `If-Modified-Since`) of a GET/HEAD against
/// the resource's CURRENT validators (RFC 9110 §13).
///
/// The caller has already (a) authorized the read and (b) confirmed the resource EXISTS (a missing
/// target is a 404 *before* this point, so no 304 can leak the existence of a resource the caller
/// could not otherwise read). `current_etag` is therefore the ETag a 200 would carry for this exact
/// representation state — the 304 uses the IDENTICAL tag.
///
/// Precedence (RFC 9110 §13.1.3): **`If-None-Match` takes precedence over `If-Modified-Since`.** When
/// `If-None-Match` is present, `If-Modified-Since` is IGNORED entirely (evaluated only as a fallback
/// when `If-None-Match` is absent).
///
/// - **`If-None-Match`** uses WEAK comparison for GET/HEAD (§13.1.2): a `W/`-prefixed inbound tag
///   matches by its opaque value, and `*` matches the (existing) resource. A match ⇒ 304. The
///   opaque comparison is EXACT — `current_etag` is the negotiated representation's own tag
///   (variant suffix included), so a tag for a DIFFERENT negotiated representation of the same
///   state never produces a 304 (the client does not hold this representation).
/// - **`If-Modified-Since`** (only when `If-None-Match` is absent): if the resource's `last_modified`
///   is `≤` the header date, the representation has NOT changed ⇒ 304. When the store surfaces no
///   modification time (`last_modified` is `None`) or the date is unparseable, the condition CANNOT be
///   proven "not modified", so we fail OPEN to a fresh 200 — never a spurious 304.
pub fn evaluate_read(
    if_none_match: Option<&str>,
    if_modified_since: Option<&str>,
    current_etag: &str,
    last_modified: Option<SystemTime>,
) -> ReadPrecondition {
    // --- If-None-Match takes precedence (§13.1.3): if present it decides, and If-Modified-Since is
    // ignored. Weak comparison (§13.1.2); `*` matches the existing resource.
    if let Some(inm) = if_none_match {
        let matched = if is_wildcard(inm) {
            // The resource exists (the read reached it post-auth), so `*` matches.
            true
        } else {
            tag_list(inm).any(|t| t.matches_weak(current_etag))
        };
        return if matched {
            ReadPrecondition::NotModified
        } else {
            ReadPrecondition::Proceed
        };
    }

    // --- If-Modified-Since (ONLY when If-None-Match is absent). 304 iff the resource's Last-Modified
    // is ≤ the client's date (the representation has not changed since). Missing/unparseable ⇒ Proceed.
    if let (Some(ims), Some(lm)) = (if_modified_since, last_modified) {
        if let (Some(ims_secs), Some(lm_secs)) = (parse_http_date(ims), system_time_to_unix(lm)) {
            if lm_secs <= ims_secs {
                return ReadPrecondition::NotModified;
            }
        }
    }

    ReadPrecondition::Proceed
}

/// The `If-Unmodified-Since` date, in seconds since the Unix epoch, when a write will actually
/// evaluate it: `If-Match` is absent and the value is a valid IMF-fixdate. `pub(crate)` so the V4
/// existence-non-disclosure guard can Read-gate exactly the requests whose outcome would depend on
/// the target's modification time.
pub(crate) fn unmodified_since_applies(
    if_match: Option<&str>,
    if_unmodified_since: Option<&str>,
) -> Option<i64> {
    if if_match.is_some() {
        return None;
    }
    if_unmodified_since.and_then(|v| parse_any_http_date(v, crate::clock::now()))
}

/// Parse any of the three HTTP-date forms a recipient MUST accept (RFC 9110 §5.6.7): IMF-fixdate
/// ([`parse_http_date`]), the obsolete RFC 850 form `Sunday, 06-Nov-94 08:49:37 GMT`, and the
/// obsolete asctime form `Sun Nov  6 08:49:37 1994` (UTC). Each form is fixed-width and checked
/// strictly, calendar and weekday included.
///
/// Used where ignoring a valid date would be UNSAFE: a write's `If-Unmodified-Since` that failed to
/// parse would let a lost update through. (The read path keeps the IMF-only parser, where an ignored
/// date only costs a cache miss.) An RFC 850 two-digit year that would be more than 50 years after
/// `now` is taken as the most recent past year with those digits, as §5.6.7 requires.
fn parse_any_http_date(value: &str, now: SystemTime) -> Option<i64> {
    parse_http_date(value)
        .or_else(|| parse_rfc850_date(value, now))
        .or_else(|| parse_asctime_date(value))
}

/// `Sunday, 06-Nov-94 08:49:37 GMT`: a full day-name, then a fixed-width 22-byte tail.
fn parse_rfc850_date(value: &str, now: SystemTime) -> Option<i64> {
    let v = value.trim();
    if !v.is_ascii() {
        return None;
    }
    let (day_name, rest) = v.split_once(", ")?;
    let b = rest.as_bytes();
    if b.len() != 22
        || b[2] != b'-'
        || b[6] != b'-'
        || b[9] != b' '
        || b[12] != b':'
        || b[15] != b':'
        || &b[18..22] != b" GMT"
    {
        return None;
    }
    let day = two_digits(b, 0)?;
    let month = month_from_abbrev(&rest[3..6])?;
    let yy = i64::from(two_digits(b, 7)?);
    let (hh, mm, ss) = (two_digits(b, 10)?, two_digits(b, 13)?, two_digits(b, 16)?);
    // §5.6.7: a timestamp that appears MORE than 50 years in the future means the most recent past
    // year with the same last two digits. "50 years" is calendar years from `now`, compared on the
    // full timestamp, so the boundary year resolves by month, day and time too.
    let now_secs = system_time_to_unix(now).unwrap_or(0);
    let (now_year, now_month, now_day) = civil_from_days(now_secs.div_euclid(86_400));
    let horizon_day = now_day.min(days_in_month(now_year + 50, now_month));
    let horizon = days_from_civil(now_year + 50, now_month, horizon_day) * 86_400
        + now_secs.rem_euclid(86_400);
    let mut year = now_year - now_year.rem_euclid(100) + yy;
    if civil_to_unix(year, month, day, hh, mm, ss).is_some_and(|(secs, _)| secs > horizon) {
        year -= 100;
    }
    let (secs, days) = civil_to_unix(year, month, day, hh, mm, ss)?;
    let short = day_name_for(days);
    (day_name.len() > 3 && day_name.starts_with(short) && full_day_name(short) == day_name)
        .then_some(secs)
}

/// `Sun Nov  6 08:49:37 1994`: 24 bytes, the day of month space-padded to two characters.
fn parse_asctime_date(value: &str) -> Option<i64> {
    let v = value.trim();
    if !v.is_ascii() {
        return None;
    }
    let b = v.as_bytes();
    if b.len() != 24
        || b[3] != b' '
        || b[7] != b' '
        || b[10] != b' '
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b' '
    {
        return None;
    }
    let month = month_from_abbrev(&v[4..7])?;
    let day = match (b[8], b[9]) {
        (b' ', d) if d.is_ascii_digit() => u32::from(d - b'0'),
        (b' ', _) => return None,
        _ => two_digits(b, 8)?,
    };
    let (secs, days) = civil_to_unix(
        four_digits(b, 20)?,
        month,
        day,
        two_digits(b, 11)?,
        two_digits(b, 14)?,
        two_digits(b, 17)?,
    )?;
    (day_name_for(days) == &v[0..3]).then_some(secs)
}

/// Validate a civil UTC date-time and return (seconds, days) since the Unix epoch.
///
/// A leap second (`23:59:60`, RFC 5322 §3.3, which HTTP-date inherits) is accepted and counts as the
/// following midnight, the instant it precedes on the POSIX timeline; `:60` at any other minute is
/// not a real UTC time and is rejected.
fn civil_to_unix(year: i64, month: u32, day: u32, hh: u32, mm: u32, ss: u32) -> Option<(i64, i64)> {
    let leap_second = ss == 60 && hh == 23 && mm == 59;
    if day == 0
        || day > days_in_month(year, month)
        || hh > 23
        || mm > 59
        || (ss > 59 && !leap_second)
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some((
        days * 86_400 + i64::from(hh) * 3_600 + i64::from(mm) * 60 + i64::from(ss),
        days,
    ))
}

/// The proleptic Gregorian (year, month, day) of `days` since the Unix epoch (the inverse of
/// [`days_from_civil`], same era arithmetic).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The RFC 850 full day-name for an IMF-fixdate abbreviation.
fn full_day_name(short: &str) -> &'static str {
    match short {
        "Mon" => "Monday",
        "Tue" => "Tuesday",
        "Wed" => "Wednesday",
        "Thu" => "Thursday",
        "Fri" => "Friday",
        "Sat" => "Saturday",
        _ => "Sunday",
    }
}

/// Whether a header value is the wildcard `*` (after trimming). `pub(crate)` so the V4
/// existence-non-disclosure guard ([`crate::ldp::handler`]) can distinguish a bare `*` precondition
/// (existence-only, no content-derived ETag) from a concrete-validator one when deciding whether a
/// conditional write must be Read-gated.
pub(crate) fn is_wildcard(header: &str) -> bool {
    header.trim() == "*"
}

/// A [`SystemTime`] as whole seconds since the Unix epoch, or `None` for a pre-epoch time (which no
/// stored resource realistically has — treated as "cannot compare" ⇒ the caller fails open).
fn system_time_to_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Parse an HTTP **IMF-fixdate** (RFC 9110 §5.6.7 — the format a sender MUST produce), e.g.
/// `Sun, 06 Nov 1994 08:49:37 GMT`, to whole seconds since the Unix epoch.
///
/// STRICT: the grammar is fixed-width (exactly 29 ASCII bytes after trimming), every field is
/// enforced at its exact position and width (two-digit day/time components, four-digit year, the
/// literal `", "` / `" "` / `":"` separators, the `GMT` zone), the calendar is validated for real
/// (day within the month's true length, leap years included), and the leading day-name must be the
/// ACTUAL weekday of the date (a wrong or arbitrary day-name token is malformed — RFC 9110 §5.6.7
/// fixes `day-name` to the day of week). Anything else — the obsolete RFC 850 / asctime forms,
/// non-fixed-width fields, negative components, a `:60` second anywhere but `23:59` — returns `None`, and the
/// caller FAILS OPEN to a fresh 200 (never a spurious 304), so strictness costs at most a cache
/// miss.
///
/// No third-party date crate is pulled in for this: the grammar is fixed-width and the epoch
/// conversion is the standard days-from-civil algorithm, so a small self-contained parser is exact
/// and keeps the dependency surface unchanged.
fn parse_http_date(value: &str) -> Option<i64> {
    // `Sun, 06 Nov 1994 08:49:37 GMT` — positions:
    //  0123456789012345678901234567 8
    //  Sun ,  06 Nov  1994 08:49:37  GMT
    let v = value.trim();
    if !v.is_ascii() {
        return None; // also guarantees the byte slicing below is char-safe
    }
    let b = v.as_bytes();
    if b.len() != 29
        || &b[3..5] != b", "
        || b[7] != b' '
        || b[11] != b' '
        || b[16] != b' '
        || b[19] != b':'
        || b[22] != b':'
        || &b[25..29] != b" GMT"
    {
        return None;
    }
    let day = two_digits(b, 5)?;
    let month = month_from_abbrev(&v[8..11])?;
    let year = four_digits(b, 12)?;
    let (secs, days) = civil_to_unix(
        year,
        month,
        day,
        two_digits(b, 17)?,
        two_digits(b, 20)?,
        two_digits(b, 23)?,
    )?;
    if day_name_for(days) != &v[0..3] {
        return None; // day-name inconsistent with the actual date (or not a day-name at all)
    }
    Some(secs)
}

/// Exactly two ASCII digits at `b[at..at+2]`, as a number.
fn two_digits(b: &[u8], at: usize) -> Option<u32> {
    let (d1, d2) = (b[at], b[at + 1]);
    if d1.is_ascii_digit() && d2.is_ascii_digit() {
        Some(u32::from(d1 - b'0') * 10 + u32::from(d2 - b'0'))
    } else {
        None
    }
}

/// Exactly four ASCII digits at `b[at..at+4]`, as a year.
fn four_digits(b: &[u8], at: usize) -> Option<i64> {
    let mut acc: i64 = 0;
    for &d in &b[at..at + 4] {
        if !d.is_ascii_digit() {
            return None;
        }
        acc = acc * 10 + i64::from(d - b'0');
    }
    Some(acc)
}

/// The true length of `month` in `year` (proleptic Gregorian, leap years included).
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// The IMF-fixdate day-name for a day count since the Unix epoch (1970-01-01 was a Thursday).
fn day_name_for(days_since_epoch: i64) -> &'static str {
    const NAMES: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    NAMES[days_since_epoch.rem_euclid(7) as usize]
}

/// Map a three-letter English month abbreviation to its 1-based number.
fn month_from_abbrev(m: &str) -> Option<u32> {
    Some(match m {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

/// Days from the Unix epoch (1970-01-01) to `y-m-d` (proleptic Gregorian), by Howard Hinnant's
/// `days_from_civil` algorithm. Exact for all civil dates; negative for dates before the epoch.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// A syntactically VALID inbound entity-tag with its validator strength preserved (RFC 9110
/// §8.8.3). Only [`parse_entity_tag`] constructs one — a malformed header member never becomes an
/// `InboundTag`, so it can never reach a comparison.
struct InboundTag<'a> {
    /// The tag's inner opaque characters (the `*etagc` between the quotes, e.g. `abc` for `"abc"`),
    /// with the quotes and any `W/` prefix removed.
    opaque: &'a str,
    /// Whether the inbound validator was weak (`W/`-prefixed).
    weak: bool,
}

impl InboundTag<'_> {
    /// STRONG comparison on the STATE part (for the WRITE path's `If-Match`): both validators must
    /// be strong and the state parts equal — a `+<variant>` tag from a content-negotiated GET
    /// matches the stored state it derives from (the GET → `If-Match` round-trip). The server's
    /// stored tag is always strong, so a weak inbound tag never matches strongly.
    fn matches_strong_state(&self, current_strong: &str) -> bool {
        if self.weak {
            return false;
        }
        match parse_current(current_strong) {
            Some(cur) => state_part(self.opaque) == state_part(cur),
            None => false,
        }
    }

    /// WEAK comparison on the STATE part (for the WRITE path's `If-None-Match` guard): state parts
    /// equal regardless of strength.
    fn matches_weak_state(&self, current_strong: &str) -> bool {
        match parse_current(current_strong) {
            Some(cur) => state_part(self.opaque) == state_part(cur),
            None => false,
        }
    }

    /// WEAK comparison, EXACT opaque tags (for the READ path's `If-None-Match`): the inbound tag
    /// must equal the negotiated representation's own tag — variant suffix included — so a tag for
    /// a different negotiated representation of the same state never yields a 304.
    fn matches_weak(&self, current_strong: &str) -> bool {
        match parse_current(current_strong) {
            Some(cur) => self.opaque == cur,
            None => false,
        }
    }
}

/// Parse ONE inbound entity-tag STRICTLY per RFC 9110 §8.8.3 — `entity-tag = [ weak ] opaque-tag`,
/// `weak = "W/"` (case-sensitive), `opaque-tag = DQUOTE *etagc DQUOTE`,
/// `etagc = %x21 / %x23-7E / obs-text` (no DQUOTE, no whitespace, no control chars inside).
///
/// Returns the tag's INNER opaque characters (quotes stripped) + its strength, or `None` when the
/// value is malformed — an unclosed quote (`"abc`), a doubled trailing quote (`"abc""`), a bare
/// unquoted token, an interior DQUOTE/space/control byte, a lone `W/`, etc. Validation happens
/// BEFORE any comparison, so a malformed validator can never match anything (in particular it can
/// never be "normalised" into equality with a valid current tag — the regression roborev caught).
fn parse_entity_tag(raw: &str) -> Option<InboundTag<'_>> {
    let t = raw.trim();
    let (weak, quoted) = match t.strip_prefix("W/") {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let inner = quoted.strip_prefix('"')?.strip_suffix('"')?;
    inner
        .bytes()
        .all(|b| b == 0x21 || (0x23..=0x7E).contains(&b) || b >= 0x80)
        .then_some(InboundTag {
            opaque: inner,
            weak,
        })
}

/// The inner opaque characters of the SERVER's own current ETag, which is always a well-formed
/// STRONG tag. `None` (⇒ no match, fail closed) if it is somehow not — defensive only.
fn parse_current(current_strong: &str) -> Option<&str> {
    let tag = parse_entity_tag(current_strong)?;
    (!tag.weak).then_some(tag.opaque)
}

/// Mint the representation-variant entity-tag for a content-negotiated response: the stored (state)
/// tag with `+<variant>` appended INSIDE the quotes — `"7-1a2b"` + `jsonld` ⇒ `"7-1a2b+jsonld"`.
/// Stays a valid strong opaque-tag (RFC 9110 §8.8.3: `+` and `/` are legal `etagc`). The base tags
/// this server mints (`"<len>-<hash>"`) never contain `+`, so the suffix parses back unambiguously
/// (see the private `state_part`).
pub fn variant_etag(stored: &str, variant: &str) -> String {
    match stored.strip_suffix('"') {
        Some(head) => format!("{head}+{variant}\""),
        // Defensive: an unquoted tag (never minted by this server) still gets a distinct value.
        None => format!("{stored}+{variant}"),
    }
}

/// The STATE part of a VALIDATED tag's inner opaque characters: everything before the first `+`
/// (dropping any `+<variant>` representation suffix). Operates only on [`parse_entity_tag`] output,
/// so no quote handling happens here — a malformed tag was already rejected upstream.
fn state_part(inner: &str) -> &str {
    match inner.split_once('+') {
        Some((head, _)) => head,
        None => inner,
    }
}

/// Iterate the WELL-FORMED entity-tags in a comma-separated `If-(None-)Match` header value,
/// preserving each tag's validator STRENGTH (so `If-Match` can correctly reject a weak validator).
/// Each member is validated by [`parse_entity_tag`]; a malformed member is DROPPED — it can never
/// match — while the remaining valid members are still evaluated. (A quoted tag containing a comma
/// — legal `etagc`, never minted by this server — splits into two malformed halves and is likewise
/// dropped: fail-safe in both directions, `If-Match` won't proceed on it and `If-None-Match` won't
/// 304/412 on it. A `*` is handled by the callers on the WHOLE header only, per the grammar —
/// a `*` member inside a list is not a wildcard and, unquoted, drops as malformed.)
fn tag_list(header: &str) -> impl Iterator<Item = InboundTag<'_>> {
    header.split(',').filter_map(parse_entity_tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: &str = "\"abc-123\"";
    const OTHER: &str = "\"xyz-999\"";

    #[test]
    fn no_preconditions_always_proceeds() {
        assert_eq!(evaluate(None, None, Some(TAG)), Precondition::Proceed);
        assert_eq!(evaluate(None, None, None), Precondition::Proceed);
    }

    #[test]
    fn if_none_match_star_creates_only_when_absent() {
        // Create guard: proceed when the resource does NOT exist.
        assert_eq!(evaluate(None, Some("*"), None), Precondition::Proceed);
        // Fail when it already exists (no-overwrite create).
        assert_eq!(evaluate(None, Some("*"), Some(TAG)), Precondition::Failed);
    }

    #[test]
    fn if_match_star_requires_existence() {
        assert_eq!(evaluate(Some("*"), None, Some(TAG)), Precondition::Proceed);
        assert_eq!(evaluate(Some("*"), None, None), Precondition::Failed);
    }

    #[test]
    fn if_match_matches_current_tag() {
        assert_eq!(evaluate(Some(TAG), None, Some(TAG)), Precondition::Proceed);
        assert_eq!(evaluate(Some(OTHER), None, Some(TAG)), Precondition::Failed);
        // If-Match against a missing resource never matches.
        assert_eq!(evaluate(Some(TAG), None, None), Precondition::Failed);
    }

    #[test]
    fn if_none_match_concrete_tag_blocks_on_match() {
        // The tag matches ⇒ "none match" fails.
        assert_eq!(evaluate(None, Some(TAG), Some(TAG)), Precondition::Failed);
        // A different tag ⇒ proceeds.
        assert_eq!(
            evaluate(None, Some(OTHER), Some(TAG)),
            Precondition::Proceed
        );
    }

    #[test]
    fn if_match_list_matches_a_strong_member() {
        // A strong member of the list matches ⇒ If-Match proceeds (strong comparison).
        let list = format!("{OTHER}, {TAG}");
        assert_eq!(
            evaluate(Some(&list), None, Some(TAG)),
            Precondition::Proceed
        );
    }

    #[test]
    fn if_match_rejects_a_weak_validator() {
        // RFC 9110 §13.1.1: a weak (`W/`) validator must NOT satisfy If-Match — even with the same
        // opaque tag, so this must FAIL (the bug roborev flagged).
        let weak = format!("W/{TAG}");
        assert_eq!(evaluate(Some(&weak), None, Some(TAG)), Precondition::Failed);
        // …and a list whose only matching tag is weak still fails.
        let list = format!("{OTHER}, W/{TAG}");
        assert_eq!(evaluate(Some(&list), None, Some(TAG)), Precondition::Failed);
    }

    #[test]
    fn if_none_match_accepts_a_weak_validator() {
        // RFC 9110 §13.1.2: If-None-Match uses WEAK comparison — a `W/`-prefixed tag with the same
        // opaque value DOES match (so "none match" fails ⇒ 412 on a mutation).
        let weak = format!("W/{TAG}");
        assert_eq!(evaluate(None, Some(&weak), Some(TAG)), Precondition::Failed);
    }

    #[test]
    fn if_unmodified_since_fails_only_when_modified_later() {
        // 2000s since the epoch.
        let date = Some("Thu, 01 Jan 1970 00:33:20 GMT");
        let w = |date, lm: Option<u64>| evaluate_write(None, None, date, Some(TAG), lm.map(at));
        assert_eq!(w(date, Some(3000)), Precondition::Failed);
        assert_eq!(w(date, Some(2000)), Precondition::Proceed); // equal is "unmodified"
        assert_eq!(w(date, Some(1000)), Precondition::Proceed);
        // Sub-second precision below the date's resolution does not count as a later change.
        let just_after = UNIX_EPOCH + std::time::Duration::from_millis(2_000_900);
        assert_eq!(
            evaluate_write(None, None, date, Some(TAG), Some(just_after)),
            Precondition::Proceed
        );
        // Ignored: no modification time, a missing target, an invalid date.
        assert_eq!(w(date, None), Precondition::Proceed);
        assert_eq!(
            evaluate_write(None, None, date, None, None),
            Precondition::Proceed
        );
        assert_eq!(w(Some("yesterday"), Some(3000)), Precondition::Proceed);
    }

    #[test]
    fn if_match_takes_precedence_over_if_unmodified_since() {
        let date = Some("Thu, 01 Jan 1970 00:33:20 GMT");
        // A matching If-Match proceeds even though the target changed after the date.
        assert_eq!(
            evaluate_write(Some(TAG), None, date, Some(TAG), Some(at(3000))),
            Precondition::Proceed
        );
        assert_eq!(unmodified_since_applies(Some("*"), date), None);
        assert_eq!(unmodified_since_applies(None, date), Some(2000));
        assert_eq!(unmodified_since_applies(None, Some("bad")), None);
    }

    #[test]
    fn obsolete_http_date_forms_are_accepted_for_writes() {
        // RFC 9110 §5.6.7's own example, in all three forms.
        let now = at(1_800_000_000); // 2027
        for v in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(parse_any_http_date(v, now), Some(784_111_777), "{v}");
        }
        assert_eq!(
            parse_any_http_date("Thu Jan  1 00:33:20 1970", now),
            Some(2000)
        );
        // asctime allows a zero-padded day as well as a space-padded one.
        assert_eq!(
            parse_any_http_date("Sun Nov 06 08:49:37 1994", now),
            Some(784_111_777)
        );
        // Wrong weekdays, a missing pad, a short day-name and an impossible date are rejected.
        for v in [
            "Monday, 06-Nov-94 08:49:37 GMT",
            "Sun, 06-Nov-94 08:49:37 GMT",
            "Mon Nov  6 08:49:37 1994",
            "Sun Nov 6 08:49:37 1994",
            "Thursday, 31-Nov-94 08:49:37 GMT",
        ] {
            assert_eq!(parse_any_http_date(v, now), None, "{v}");
        }
        // Two-digit years more than 50 years ahead belong to the previous century.
        let in_2027 = parse_any_http_date("Wednesday, 01-Jan-76 00:00:00 GMT", now).unwrap();
        assert_eq!(
            in_2027,
            parse_http_date("Wed, 01 Jan 2076 00:00:00 GMT").unwrap()
        );
        let late = at(4_102_444_800); // 2100-01-01
        assert_eq!(
            parse_any_http_date("Saturday, 06-Nov-94 08:49:37 GMT", late),
            parse_http_date("Sat, 06 Nov 2094 08:49:37 GMT")
        );
    }

    #[test]
    fn rfc850_century_cutoff_compares_the_full_timestamp() {
        // Now: 2026-10-14 10:00:00 UTC. The cutoff is 2076-10-14 10:00:00.
        let now = at(1_791_972_000);
        let date = |v| parse_any_http_date(v, now);
        // December 2076 is past the cutoff, so `76` there means 1976.
        assert_eq!(
            date("Friday, 31-Dec-76 12:00:00 GMT"),
            parse_http_date("Fri, 31 Dec 1976 12:00:00 GMT")
        );
        assert_eq!(date("Thursday, 31-Dec-76 12:00:00 GMT"), None);
        // The cutoff instant itself is exactly 50 years ahead, not more: it stays in 2076.
        assert_eq!(
            date("Wednesday, 14-Oct-76 10:00:00 GMT"),
            Some(3_369_895_200)
        );
        // One second later it rolls back a century (and must then carry 1976's weekday).
        assert_eq!(
            date("Thursday, 14-Oct-76 10:00:01 GMT"),
            parse_http_date("Thu, 14 Oct 1976 10:00:01 GMT")
        );
    }

    #[test]
    fn a_leap_second_is_the_following_midnight_in_every_form() {
        let now = at(1_791_972_000);
        let midnight = Some(1_483_228_800); // 2017-01-01T00:00:00Z
        for v in [
            "Sat, 31 Dec 2016 23:59:60 GMT",
            "Saturday, 31-Dec-16 23:59:60 GMT",
            "Sat Dec 31 23:59:60 2016",
        ] {
            assert_eq!(parse_any_http_date(v, now), midnight, "{v}");
        }
        assert_eq!(parse_http_date("Sat, 31 Dec 2016 23:58:60 GMT"), None);
        assert_eq!(parse_http_date("Sat, 31 Dec 2016 23:59:61 GMT"), None);
    }

    #[test]
    fn if_none_match_still_applies_after_if_unmodified_since() {
        let date = Some("Thu, 01 Jan 1970 00:33:20 GMT");
        assert_eq!(
            evaluate_write(None, Some("*"), date, Some(TAG), Some(at(1000))),
            Precondition::Failed
        );
    }

    #[test]
    fn require_maps_to_status() {
        assert!(require(Precondition::Proceed).is_ok());
        let err = require(Precondition::Failed).unwrap_err();
        assert_eq!(err.status().as_u16(), 412);
    }

    // --- READ preconditions (GET/HEAD → 304) -------------------------------------------------------

    use std::time::Duration;

    /// A `SystemTime` `secs` seconds after the Unix epoch.
    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn read_no_preconditions_proceeds() {
        assert_eq!(
            evaluate_read(None, None, TAG, None),
            ReadPrecondition::Proceed
        );
        assert_eq!(
            evaluate_read(None, None, TAG, Some(at(1_000))),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn read_if_none_match_matching_is_304() {
        // A matching tag ⇒ the client's cache is fresh ⇒ 304.
        assert_eq!(
            evaluate_read(Some(TAG), None, TAG, None),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_none_match_non_matching_is_200() {
        assert_eq!(
            evaluate_read(Some(OTHER), None, TAG, None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn read_if_none_match_star_on_existing_is_304() {
        // `If-None-Match: *` on a resource that exists (the read reached it) ⇒ 304.
        assert_eq!(
            evaluate_read(Some("*"), None, TAG, None),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_none_match_weak_validator_matches() {
        // GET/HEAD use WEAK comparison (§13.1.2): a `W/`-prefixed inbound tag matches the strong
        // current tag by opaque value ⇒ 304.
        let weak = format!("W/{TAG}");
        assert_eq!(
            evaluate_read(Some(&weak), None, TAG, None),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_none_match_list_matches_any_member() {
        let list = format!("{OTHER}, {TAG}");
        assert_eq!(
            evaluate_read(Some(&list), None, TAG, None),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_modified_since_not_modified_is_304() {
        // Resource last modified at t=1000; client asks "modified since t=2000?" → no ⇒ 304.
        let ims = "Thu, 01 Jan 1970 00:33:20 GMT"; // 2000s after epoch
        assert_eq!(
            evaluate_read(None, Some(ims), TAG, Some(at(1_000))),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_modified_since_modified_is_200() {
        // Resource last modified at t=3000; client asks "modified since t=2000?" → yes ⇒ 200.
        let ims = "Thu, 01 Jan 1970 00:33:20 GMT"; // 2000s
        assert_eq!(
            evaluate_read(None, Some(ims), TAG, Some(at(3_000))),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn read_if_modified_since_equal_boundary_is_304() {
        // Last-Modified EXACTLY equal to the date ⇒ "not modified since" is inclusive ⇒ 304.
        let ims = "Thu, 01 Jan 1970 00:33:20 GMT"; // 2000s
        assert_eq!(
            evaluate_read(None, Some(ims), TAG, Some(at(2_000))),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_none_match_present_suppresses_if_modified_since() {
        // Precedence (§13.1.3): a NON-matching If-None-Match ⇒ Proceed, EVEN with an
        // If-Modified-Since that would otherwise 304. If-None-Match decides; If-Modified-Since ignored.
        let ims = "Thu, 01 Jan 1970 00:33:20 GMT"; // 2000s — would be "not modified" for lm=1000
        assert_eq!(
            evaluate_read(Some(OTHER), Some(ims), TAG, Some(at(1_000))),
            ReadPrecondition::Proceed
        );
        // …and a MATCHING If-None-Match still 304s regardless of the (contradictory) IMS.
        assert_eq!(
            evaluate_read(Some(TAG), Some(ims), TAG, Some(at(3_000))),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_modified_since_without_last_modified_proceeds() {
        // No stored modification time ⇒ cannot prove "not modified" ⇒ fail OPEN to a fresh 200.
        let ims = "Thu, 01 Jan 1970 00:33:20 GMT";
        assert_eq!(
            evaluate_read(None, Some(ims), TAG, None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn read_if_modified_since_unparseable_proceeds() {
        assert_eq!(
            evaluate_read(None, Some("not-a-date"), TAG, Some(at(1_000))),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn parse_http_date_known_values() {
        // The canonical RFC 9110 example: 784111777 seconds since the epoch.
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        // The epoch itself.
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        // A round-2000s value used by the tests above.
        assert_eq!(
            parse_http_date("Thu, 01 Jan 1970 00:33:20 GMT"),
            Some(2_000)
        );
    }

    #[test]
    fn parse_http_date_rejects_malformed() {
        assert_eq!(parse_http_date(""), None);
        assert_eq!(parse_http_date("garbage"), None);
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 UTC"), None); // not GMT
        assert_eq!(parse_http_date("Sun, 32 Nov 1994 08:49:37 GMT"), None); // bad day
        assert_eq!(parse_http_date("Sun, 06 Foo 1994 08:49:37 GMT"), None); // bad month
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 25:49:37 GMT"), None); // bad hour
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT extra"), None);
        // trailing junk
    }

    #[test]
    fn parse_http_date_enforces_strict_imf_fixdate_grammar() {
        // Non-fixed-width day (single digit) — the grammar is exactly 2DIGIT.
        assert_eq!(parse_http_date("Sun, 6 Nov 1994 08:49:37 GMT"), None);
        // Negative / non-digit components.
        assert_eq!(parse_http_date("Sun, -6 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:4x:37 GMT"), None);
        // The obsolete RFC 850 / two-digit-year / asctime forms.
        assert_eq!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 Nov 94 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Sun Nov  6 08:49:37 1994"), None);
        // Wrong separators at the fixed positions.
        assert_eq!(parse_http_date("Sun; 06 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08-49-37 GMT"), None);
        // Case-sensitive tokens.
        assert_eq!(parse_http_date("sun, 06 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 NOV 1994 08:49:37 GMT"), None);
        // `:60` outside 23:59 is not a real leap second and is rejected (fails open to a 200).
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:60 GMT"), None);
        // Non-ASCII input of plausible shape must not panic (byte-slicing guard) and must fail.
        assert_eq!(parse_http_date("Sün, 06 Nov 1994 08:49:37 GMT"), None);
    }

    #[test]
    fn parse_http_date_validates_the_real_calendar() {
        // Day zero / day beyond the month's true length.
        assert_eq!(parse_http_date("Wed, 00 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Thu, 31 Nov 1994 08:49:37 GMT"), None); // Nov has 30 days
        // Feb 29 exists only in a leap year.
        assert_eq!(parse_http_date("Mon, 29 Feb 1999 00:00:00 GMT"), None);
        assert_eq!(parse_http_date("Tue, 29 Feb 1900 00:00:00 GMT"), None); // century, not leap
        assert_eq!(
            parse_http_date("Tue, 29 Feb 2000 00:00:00 GMT"), // 400-year rule: leap
            Some(951_782_400)
        );
    }

    #[test]
    fn parse_http_date_requires_the_correct_weekday() {
        // 1994-11-06 was a Sunday: the correct day-name parses…
        assert!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").is_some());
        // …a real-but-WRONG day-name is malformed…
        assert_eq!(parse_http_date("Mon, 06 Nov 1994 08:49:37 GMT"), None);
        // …and an arbitrary token is malformed.
        assert_eq!(parse_http_date("Xyz, 06 Nov 1994 08:49:37 GMT"), None);
    }

    // --- Representation-variant tags (`"<state>+<variant>"`) ---------------------------------------

    /// The JSON-LD variant of [`TAG`], as the handler mints for a content-negotiated response.
    fn tag_variant() -> String {
        variant_etag(TAG, "jsonld")
    }

    #[test]
    fn variant_etag_appends_inside_the_quotes() {
        assert_eq!(variant_etag("\"7-1a2b\"", "jsonld"), "\"7-1a2b+jsonld\"");
        assert_eq!(variant_etag("\"7-1a2b\"", "ttl"), "\"7-1a2b+ttl\"");
        // Defensive unquoted fallback still yields a distinct value.
        assert_eq!(variant_etag("bare", "jsonld"), "bare+jsonld");
    }

    #[test]
    fn write_if_match_accepts_a_variant_tag_of_the_current_state() {
        // GET (Accept: JSON-LD) → variant tag → If-Match PUT with it must round-trip: the state is
        // unchanged, so the write proceeds.
        assert_eq!(
            evaluate(Some(&tag_variant()), None, Some(TAG)),
            Precondition::Proceed
        );
    }

    #[test]
    fn write_if_match_rejects_a_variant_of_a_stale_state() {
        // The variant derives from a DIFFERENT (old) state ⇒ 412.
        let stale = variant_etag(OTHER, "jsonld");
        assert_eq!(
            evaluate(Some(&stale), None, Some(TAG)),
            Precondition::Failed
        );
    }

    #[test]
    fn write_if_match_still_rejects_a_weak_variant_tag() {
        // Strong comparison: a weak validator never satisfies If-Match, variant or not.
        let weak = format!("W/{}", tag_variant());
        assert_eq!(evaluate(Some(&weak), None, Some(TAG)), Precondition::Failed);
    }

    #[test]
    fn write_if_none_match_guard_matches_a_variant_of_the_current_state() {
        // The no-overwrite guard: a variant of the CURRENT state is "current" ⇒ Failed (412).
        assert_eq!(
            evaluate(None, Some(&tag_variant()), Some(TAG)),
            Precondition::Failed
        );
    }

    #[test]
    fn read_comparison_is_exact_per_representation() {
        // The read path compares EXACT opaque tags against the negotiated representation's own
        // validator. A client holding the STORED (Turtle) tag asking for the JSON-LD variant does
        // NOT hold that representation ⇒ Proceed (200), never 304.
        let served_variant = tag_variant();
        assert_eq!(
            evaluate_read(Some(TAG), None, &served_variant, None),
            ReadPrecondition::Proceed
        );
        // And vice versa: a variant tag never 304s the stored representation.
        assert_eq!(
            evaluate_read(Some(&served_variant), None, TAG, None),
            ReadPrecondition::Proceed
        );
        // Holding the variant tag AND being served that variant ⇒ 304.
        assert_eq!(
            evaluate_read(Some(&served_variant), None, &served_variant, None),
            ReadPrecondition::NotModified
        );
    }

    // --- Entity-tag syntax validation (RFC 9110 §8.8.3) — malformed tags never match --------------

    /// Malformed entity-tag header values: each must never satisfy any precondition against the
    /// valid current tag [`TAG`] = `"abc-123"`.
    const MALFORMED: &[&str] = &[
        "\"abc-123",     // unclosed quote
        "abc-123\"",     // unopened quote
        "\"abc-123\"\"", // doubled trailing quote
        "abc-123",       // bare unquoted token
        "\"abc 123\"",   // interior space (not etagc)
        "\"abc\"123\"",  // interior DQUOTE (not etagc)
        "W/abc-123",     // weak prefix without quotes
        "W/",            // lone weak prefix
        "w/\"abc-123\"", // lowercase weak prefix (RFC 9110: `weak = %x57.2F`, case-sensitive)
    ];

    #[test]
    fn malformed_if_match_never_matches_the_valid_current_tag() {
        for bad in MALFORMED {
            assert_eq!(
                evaluate(Some(bad), None, Some(TAG)),
                Precondition::Failed,
                "If-Match {bad:?} must not match {TAG:?}"
            );
        }
        // A well-formed EMPTY tag `""` is valid syntax but matches only an empty opaque-tag —
        // never a real current tag (and this server never mints an empty tag).
        assert_eq!(
            evaluate(Some("\"\""), None, Some(TAG)),
            Precondition::Failed
        );
    }

    #[test]
    fn malformed_if_none_match_never_matches_so_the_guard_proceeds() {
        // On If-None-Match, "no match" means the request PROCEEDS (write guard passes) — a
        // malformed tag must not 412.
        for bad in MALFORMED {
            assert_eq!(
                evaluate(None, Some(bad), Some(TAG)),
                Precondition::Proceed,
                "If-None-Match {bad:?} must not match {TAG:?}"
            );
        }
        assert_eq!(
            evaluate(None, Some("\"\""), Some(TAG)),
            Precondition::Proceed
        );
    }

    #[test]
    fn malformed_read_if_none_match_is_a_fresh_200_never_304() {
        for bad in MALFORMED {
            assert_eq!(
                evaluate_read(Some(bad), None, TAG, None),
                ReadPrecondition::Proceed,
                "read If-None-Match {bad:?} must not 304 against {TAG:?}"
            );
        }
        assert_eq!(
            evaluate_read(Some("\"\""), None, TAG, None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn empty_opaque_tag_matches_only_an_empty_current_tag() {
        // `""` is well-formed (`*etagc` allows zero chars): exact-match semantics still hold.
        assert_eq!(
            evaluate_read(Some("\"\""), None, "\"\"", None),
            ReadPrecondition::NotModified
        );
        // But it is NOT the same as a tag whose CONTENT is a quote-pair — that content is not
        // valid etagc, so such an inbound tag is malformed and matches nothing.
        assert_eq!(
            evaluate_read(Some("\"\\\"\\\"\""), None, "\"\"", None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn malformed_member_in_a_list_is_dropped_but_valid_members_still_match() {
        // A malformed member must not poison the list: the VALID member still matches…
        let with_valid = format!("\"unclosed, {TAG}");
        assert_eq!(
            evaluate(Some(&with_valid), None, Some(TAG)),
            Precondition::Proceed,
            "the valid If-Match member must still match"
        );
        assert_eq!(
            evaluate_read(Some(&with_valid), None, TAG, None),
            ReadPrecondition::NotModified,
            "the valid read member must still 304"
        );
        // …and a list of ONLY malformed members matches nothing.
        let all_bad = "\"unclosed, bare, \"doubled\"\"";
        assert_eq!(
            evaluate(Some(all_bad), None, Some(TAG)),
            Precondition::Failed
        );
        assert_eq!(
            evaluate_read(Some(all_bad), None, TAG, None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn wildcard_is_only_the_whole_header_never_a_list_member() {
        // `*` inside a list is not a wildcard (the grammar is `*` OR a tag list) — and, unquoted,
        // it drops as malformed, so it cannot satisfy If-Match…
        assert_eq!(
            evaluate(Some(&format!("*, {OTHER}")), None, Some(TAG)),
            Precondition::Failed
        );
        // …nor trip If-None-Match / the read path into a match.
        assert_eq!(
            evaluate(None, Some(&format!("*, {OTHER}")), Some(TAG)),
            Precondition::Proceed
        );
        assert_eq!(
            evaluate_read(Some(&format!("*, {OTHER}")), None, TAG, None),
            ReadPrecondition::Proceed
        );
    }
}
