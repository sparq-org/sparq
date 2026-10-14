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

use crate::entity_tag::TagList;
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
/// `if_match` / `if_none_match` are the raw field values as bytes, every field line combined; a
/// value that is not an entity-tag list names no tag. `current` is `Some(etag)` if the target
/// exists with that strong ETag, or `None` if the target does not currently exist.
///
/// Precedence follows RFC 9110 §13.2.2: when both are present, `If-Match` is evaluated **first**.
/// (`If-None-Match` then still applies — but a request that supplies both a matching `If-Match` and
/// an `If-None-Match` for the same existing tag is contradictory and fails on the `If-None-Match`.)
pub fn evaluate(
    if_match: Option<&[u8]>,
    if_none_match: Option<&[u8]>,
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
    if_match: Option<&[u8]>,
    if_none_match: Option<&[u8]>,
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
        // `If-Match: *` ⇒ the resource must exist; no missing representation matches a tag.
        let ok = list_hits(im, current.is_some(), |weak, tag| {
            !weak && current_state(current).is_some_and(|cur| state_part(tag) == cur)
        });
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
        let matched = list_hits(inm, current.is_some(), |_, tag| {
            current_state(current).is_some_and(|cur| state_part(tag) == cur)
        });
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

/// Whether an If-Range precondition lets the Range apply (RFC 9110 section 13.1.5): absent, or a
/// single strong entity tag equal to the current one. A date never does: two versions written
/// within one second share a Last-Modified, so a date is not a strong validator here, and the
/// whole representation is served instead. If-Range carries one validator, so a repeated field
/// or an unreadable one never holds either. Both the LDP and the LWS surface use it.
pub(crate) fn if_range_holds(headers: &http::HeaderMap, etag: &str) -> bool {
    let mut lines = headers.get_all(http::header::IF_RANGE).iter();
    let Some(line) = lines.next() else {
        return true;
    };
    if lines.next().is_some() {
        return false;
    }
    // One strong tag, in the RFC 9110 grammar; a weak tag never matches: the comparison is strong.
    match TagList::parse(line.as_bytes()) {
        Ok(TagList::Tags(tags)) => match tags[..] {
            [(false, tag)] => parse_current(etag) == Some(tag),
            _ => false,
        },
        _ => false,
    }
}

/// Evaluate RFC 9110 §13.2.2 steps 1 and 2 for a GET/HEAD: `If-Match`, or without it
/// `If-Unmodified-Since`, before the read's own [`evaluate_read`]. `If-Match` uses EXACT strong
/// comparison against the selected representation's tag (§8.8.3.2), variant suffix included, so a
/// tag of another negotiated representation of the same state fails (a client must not combine
/// pieces of two representations); `*` passes, the resource existing. `If-Unmodified-Since` fails
/// when `last_modified` is after the date and is ignored without a time (a container listing).
pub fn evaluate_read_guard(
    if_match: Option<&[u8]>,
    if_unmodified_since: Option<&str>,
    current_etag: &str,
    last_modified: Option<SystemTime>,
) -> Precondition {
    if let Some(im) = if_match {
        let ok = list_hits(im, true, |weak, tag| {
            !weak && parse_current(current_etag) == Some(tag)
        });
        return if ok {
            Precondition::Proceed
        } else {
            Precondition::Failed
        };
    }
    match (
        unmodified_since_applies(None, if_unmodified_since),
        last_modified.and_then(system_time_to_unix),
    ) {
        (Some(date), Some(lm)) if lm > date => Precondition::Failed,
        _ => Precondition::Proceed,
    }
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
    if_none_match: Option<&[u8]>,
    if_modified_since: Option<&str>,
    current_etag: &str,
    last_modified: Option<SystemTime>,
) -> ReadPrecondition {
    // --- If-None-Match takes precedence (§13.1.3): if present it decides, and If-Modified-Since is
    // ignored. Weak comparison (§13.1.2); `*` matches the existing resource.
    if let Some(inm) = if_none_match {
        // The resource exists (the read reached it post-auth), so `*` matches.
        let matched = list_hits(inm, true, |_, tag| parse_current(current_etag) == Some(tag));
        return if matched {
            ReadPrecondition::NotModified
        } else {
            ReadPrecondition::Proceed
        };
    }

    // --- If-Modified-Since (ONLY when If-None-Match is absent). 304 iff the resource's Last-Modified
    // is ≤ the client's date (the representation has not changed since). Missing/unparseable ⇒ Proceed.
    if let (Some(ims), Some(lm)) = (if_modified_since, last_modified) {
        if let (Some(ims_secs), Some(lm_secs)) = (
            crate::http_date::parse(ims, crate::clock::now()),
            system_time_to_unix(lm),
        ) {
            if lm_secs <= ims_secs {
                return ReadPrecondition::NotModified;
            }
        }
    }

    ReadPrecondition::Proceed
}

/// The `If-Unmodified-Since` date, in seconds since the Unix epoch, when a write will actually
/// evaluate it: `If-Match` is absent and the value is a valid HTTP-date ([`crate::http_date::parse`]).
/// `pub(crate)` so the V4
/// existence-non-disclosure guard can Read-gate exactly the requests whose outcome would depend on
/// the target's modification time.
pub(crate) fn unmodified_since_applies(
    if_match: Option<&[u8]>,
    if_unmodified_since: Option<&str>,
) -> Option<i64> {
    if if_match.is_some() {
        return None;
    }
    if_unmodified_since.and_then(|v| crate::http_date::parse(v, crate::clock::now()))
}

/// A [`SystemTime`] as whole seconds since the Unix epoch, or `None` for a pre-epoch time (which no
/// stored resource realistically has — treated as "cannot compare" ⇒ the caller fails open).
fn system_time_to_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Whether the `If-Match` / `If-None-Match` field `value` names a tag `(weak, opaque)` for which
/// `hit` holds, the wildcard `*` counting as `wildcard`. The list is read by the crate's one
/// entity-tag parser ([`crate::entity_tag::TagList`]), on the field's bytes. A value that is not
/// the RFC 9110 grammar (§8.8.3) names no tag, so it matches nothing: `If-Match` fails and
/// `If-None-Match` neither 304s nor 412s. A `*` is a wildcard only as the whole value.
fn list_hits(value: &[u8], wildcard: bool, hit: impl Fn(bool, &[u8]) -> bool) -> bool {
    match TagList::parse(value) {
        Ok(TagList::Any) => wildcard,
        Ok(TagList::Tags(tags)) => tags.iter().any(|&(weak, tag)| hit(weak, tag)),
        Err(()) => false,
    }
}

/// The opaque bytes of the SERVER's own current ETag, which is always a well-formed STRONG tag.
/// `None` (⇒ no match, fail closed) if it is somehow not — defensive only.
fn parse_current(current_strong: &str) -> Option<&[u8]> {
    match TagList::parse(current_strong.as_bytes()) {
        Ok(TagList::Tags(tags)) => match tags[..] {
            [(false, opaque)] => Some(opaque),
            _ => None,
        },
        _ => None,
    }
}

/// The STATE part of the current ETag, for the write comparisons.
fn current_state(current: Option<&str>) -> Option<&[u8]> {
    current.and_then(parse_current).map(state_part)
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

/// The STATE part of a tag's opaque bytes: everything before the first `+` (dropping any
/// `+<variant>` representation suffix).
fn state_part(opaque: &[u8]) -> &[u8] {
    match opaque.iter().position(|&b| b == b'+') {
        Some(i) => &opaque[..i],
        None => opaque,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_range() {
        fn holds(lines: &[&[u8]], etag: &str) -> bool {
            let mut h = http::HeaderMap::new();
            for line in lines {
                h.append(
                    http::header::IF_RANGE,
                    http::HeaderValue::from_bytes(line).unwrap(),
                );
            }
            if_range_holds(&h, etag)
        }
        let date =
            crate::http_date::format(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
                .unwrap();
        assert!(holds(&[], "\"a\""));
        assert!(holds(&[b"\"a\""], "\"a\""));
        assert!(!holds(&[b"\"b\""], "\"a\""));
        assert!(!holds(&[b"W/\"a\""], "\"a\""));
        // Review finding: a date is not a strong validator (two versions can share one), so
        // even one equal to Last-Modified serves the whole representation.
        assert!(!holds(&[date.as_bytes()], "\"a\""));
        assert!(!holds(&[b"garbage"], "\"a\""));
        // One validator only: a repeated field holds even when a line names the current tag.
        assert!(!holds(&[b"\"a\"", b"\"a\""], "\"a\""));
        assert!(!holds(&[b"\"a\"", b"\"b\""], "\"a\""));
        // A line that is not UTF-8 is read, not skipped, so it cannot fall back to "absent".
        assert!(!holds(&[b"\"\xff\""], "\"a\""));
        // One tag in the RFC 9110 grammar: a list never holds, and only space and tab may surround it.
        assert!(!holds(&[b"\"a\", \"b\""], "\"a\""));
        assert!(holds(&[b" \"a\"\t"], "\"a\""));
        assert!(!holds(&[b"\xc2\xa0\"a\""], "\"a\""));
    }

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
        assert_eq!(
            evaluate(None, Some("*".as_bytes()), None),
            Precondition::Proceed
        );
        // Fail when it already exists (no-overwrite create).
        assert_eq!(
            evaluate(None, Some("*".as_bytes()), Some(TAG)),
            Precondition::Failed
        );
    }

    #[test]
    fn if_match_star_requires_existence() {
        assert_eq!(
            evaluate(Some("*".as_bytes()), None, Some(TAG)),
            Precondition::Proceed
        );
        assert_eq!(
            evaluate(Some("*".as_bytes()), None, None),
            Precondition::Failed
        );
    }

    #[test]
    fn if_match_matches_current_tag() {
        assert_eq!(
            evaluate(Some(TAG.as_bytes()), None, Some(TAG)),
            Precondition::Proceed
        );
        assert_eq!(
            evaluate(Some(OTHER.as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
        // If-Match against a missing resource never matches.
        assert_eq!(
            evaluate(Some(TAG.as_bytes()), None, None),
            Precondition::Failed
        );
    }

    #[test]
    fn if_none_match_concrete_tag_blocks_on_match() {
        // The tag matches ⇒ "none match" fails.
        assert_eq!(
            evaluate(None, Some(TAG.as_bytes()), Some(TAG)),
            Precondition::Failed
        );
        // A different tag ⇒ proceeds.
        assert_eq!(
            evaluate(None, Some(OTHER.as_bytes()), Some(TAG)),
            Precondition::Proceed
        );
    }

    #[test]
    fn if_match_list_matches_a_strong_member() {
        // A strong member of the list matches ⇒ If-Match proceeds (strong comparison).
        let list = format!("{OTHER}, {TAG}");
        assert_eq!(
            evaluate(Some(list.as_bytes()), None, Some(TAG)),
            Precondition::Proceed
        );
    }

    #[test]
    fn if_match_rejects_a_weak_validator() {
        // RFC 9110 §13.1.1: a weak (`W/`) validator must NOT satisfy If-Match — even with the same
        // opaque tag, so this must FAIL (the bug roborev flagged).
        let weak = format!("W/{TAG}");
        assert_eq!(
            evaluate(Some(weak.as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
        // …and a list whose only matching tag is weak still fails.
        let list = format!("{OTHER}, W/{TAG}");
        assert_eq!(
            evaluate(Some(list.as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
    }

    #[test]
    fn if_none_match_accepts_a_weak_validator() {
        // RFC 9110 §13.1.2: If-None-Match uses WEAK comparison — a `W/`-prefixed tag with the same
        // opaque value DOES match (so "none match" fails ⇒ 412 on a mutation).
        let weak = format!("W/{TAG}");
        assert_eq!(
            evaluate(None, Some(weak.as_bytes()), Some(TAG)),
            Precondition::Failed
        );
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
            evaluate_write(Some(TAG.as_bytes()), None, date, Some(TAG), Some(at(3000))),
            Precondition::Proceed
        );
        assert_eq!(unmodified_since_applies(Some("*".as_bytes()), date), None);
        assert_eq!(unmodified_since_applies(None, date), Some(2000));
        assert_eq!(unmodified_since_applies(None, Some("bad")), None);
    }

    #[test]
    fn if_none_match_still_applies_after_if_unmodified_since() {
        let date = Some("Thu, 01 Jan 1970 00:33:20 GMT");
        assert_eq!(
            evaluate_write(None, Some("*".as_bytes()), date, Some(TAG), Some(at(1000))),
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
            evaluate_read(Some(TAG.as_bytes()), None, TAG, None),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_none_match_non_matching_is_200() {
        assert_eq!(
            evaluate_read(Some(OTHER.as_bytes()), None, TAG, None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn read_if_none_match_star_on_existing_is_304() {
        // `If-None-Match: *` on a resource that exists (the read reached it) ⇒ 304.
        assert_eq!(
            evaluate_read(Some("*".as_bytes()), None, TAG, None),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_none_match_weak_validator_matches() {
        // GET/HEAD use WEAK comparison (§13.1.2): a `W/`-prefixed inbound tag matches the strong
        // current tag by opaque value ⇒ 304.
        let weak = format!("W/{TAG}");
        assert_eq!(
            evaluate_read(Some(weak.as_bytes()), None, TAG, None),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn read_if_none_match_list_matches_any_member() {
        let list = format!("{OTHER}, {TAG}");
        assert_eq!(
            evaluate_read(Some(list.as_bytes()), None, TAG, None),
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
            evaluate_read(Some(OTHER.as_bytes()), Some(ims), TAG, Some(at(1_000))),
            ReadPrecondition::Proceed
        );
        // …and a MATCHING If-None-Match still 304s regardless of the (contradictory) IMS.
        assert_eq!(
            evaluate_read(Some(TAG.as_bytes()), Some(ims), TAG, Some(at(3_000))),
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
            evaluate(Some(tag_variant().as_bytes()), None, Some(TAG)),
            Precondition::Proceed
        );
    }

    #[test]
    fn write_if_match_rejects_a_variant_of_a_stale_state() {
        // The variant derives from a DIFFERENT (old) state ⇒ 412.
        let stale = variant_etag(OTHER, "jsonld");
        assert_eq!(
            evaluate(Some(stale.as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
    }

    #[test]
    fn write_if_match_still_rejects_a_weak_variant_tag() {
        // Strong comparison: a weak validator never satisfies If-Match, variant or not.
        let weak = format!("W/{}", tag_variant());
        assert_eq!(
            evaluate(Some(weak.as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
    }

    #[test]
    fn write_if_none_match_guard_matches_a_variant_of_the_current_state() {
        // The no-overwrite guard: a variant of the CURRENT state is "current" ⇒ Failed (412).
        assert_eq!(
            evaluate(None, Some(tag_variant().as_bytes()), Some(TAG)),
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
            evaluate_read(Some(TAG.as_bytes()), None, &served_variant, None),
            ReadPrecondition::Proceed
        );
        // And vice versa: a variant tag never 304s the stored representation.
        assert_eq!(
            evaluate_read(Some(served_variant.as_bytes()), None, TAG, None),
            ReadPrecondition::Proceed
        );
        // Holding the variant tag AND being served that variant ⇒ 304.
        assert_eq!(
            evaluate_read(Some(served_variant.as_bytes()), None, &served_variant, None),
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
                evaluate(Some(bad.as_bytes()), None, Some(TAG)),
                Precondition::Failed,
                "If-Match {bad:?} must not match {TAG:?}"
            );
        }
        // A well-formed EMPTY tag `""` is valid syntax but matches only an empty opaque-tag —
        // never a real current tag (and this server never mints an empty tag).
        assert_eq!(
            evaluate(Some("\"\"".as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
    }

    #[test]
    fn malformed_if_none_match_never_matches_so_the_guard_proceeds() {
        // On If-None-Match, "no match" means the request PROCEEDS (write guard passes) — a
        // malformed tag must not 412.
        for bad in MALFORMED {
            assert_eq!(
                evaluate(None, Some(bad.as_bytes()), Some(TAG)),
                Precondition::Proceed,
                "If-None-Match {bad:?} must not match {TAG:?}"
            );
        }
        assert_eq!(
            evaluate(None, Some("\"\"".as_bytes()), Some(TAG)),
            Precondition::Proceed
        );
    }

    #[test]
    fn malformed_read_if_none_match_is_a_fresh_200_never_304() {
        for bad in MALFORMED {
            assert_eq!(
                evaluate_read(Some(bad.as_bytes()), None, TAG, None),
                ReadPrecondition::Proceed,
                "read If-None-Match {bad:?} must not 304 against {TAG:?}"
            );
        }
        assert_eq!(
            evaluate_read(Some("\"\"".as_bytes()), None, TAG, None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn empty_opaque_tag_matches_only_an_empty_current_tag() {
        // `""` is well-formed (`*etagc` allows zero chars): exact-match semantics still hold.
        assert_eq!(
            evaluate_read(Some("\"\"".as_bytes()), None, "\"\"", None),
            ReadPrecondition::NotModified
        );
        // But it is NOT the same as a tag whose CONTENT is a quote-pair — that content is not
        // valid etagc, so such an inbound tag is malformed and matches nothing.
        assert_eq!(
            evaluate_read(Some("\"\\\"\\\"\"".as_bytes()), None, "\"\"", None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn a_list_that_is_not_the_grammar_names_no_tag() {
        // An unclosed quote swallows the rest of the value, so the list cannot be read, even
        // though a well-formed tag follows: it matches nothing.
        let with_valid = format!("\"unclosed, {TAG}");
        assert_eq!(
            evaluate(Some(with_valid.as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
        assert_eq!(
            evaluate(None, Some(with_valid.as_bytes()), Some(TAG)),
            Precondition::Proceed
        );
        assert_eq!(
            evaluate_read(Some(with_valid.as_bytes()), None, TAG, None),
            ReadPrecondition::Proceed
        );
        let all_bad = "\"unclosed, bare, \"doubled\"\"";
        assert_eq!(
            evaluate(Some(all_bad.as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
        assert_eq!(
            evaluate_read(Some(all_bad.as_bytes()), None, TAG, None),
            ReadPrecondition::Proceed
        );
    }

    #[test]
    fn unicode_whitespace_around_a_validator_is_not_http_whitespace() {
        // Review finding: only space and tab may surround a list member (RFC 9110 §5.6.3). A
        // validator wrapped in other Unicode whitespace is malformed, so it matches nothing on
        // either header, read or write, and `*` so wrapped is not a wildcard.
        for ws in ["\u{a0}", "\u{3000}", "\u{85}", "\u{2028}"] {
            for v in [format!("{ws}{TAG}{ws}"), format!("{ws}*{ws}")] {
                let v = v.as_bytes();
                assert_eq!(
                    evaluate(Some(v), None, Some(TAG)),
                    Precondition::Failed,
                    "{ws:?}"
                );
                assert_eq!(
                    evaluate(None, Some(v), Some(TAG)),
                    Precondition::Proceed,
                    "{ws:?}"
                );
                assert_eq!(
                    evaluate_read_guard(Some(v), None, TAG, None),
                    Precondition::Failed,
                    "{ws:?}"
                );
                assert_eq!(
                    evaluate_read(Some(v), None, TAG, None),
                    ReadPrecondition::Proceed,
                    "{ws:?}"
                );
            }
        }
        // Space and tab still surround a member.
        let ows = format!(" \t{TAG}\t ");
        assert_eq!(
            evaluate(Some(ows.as_bytes()), None, Some(TAG)),
            Precondition::Proceed
        );
        assert_eq!(
            evaluate_read(Some(ows.as_bytes()), None, TAG, None),
            ReadPrecondition::NotModified
        );
    }

    #[test]
    fn wildcard_is_only_the_whole_header_never_a_list_member() {
        // `*` inside a list is not a wildcard (the grammar is `*` OR a tag list) — and, unquoted,
        // it makes the list unreadable, so it cannot satisfy If-Match…
        assert_eq!(
            evaluate(Some(format!("*, {OTHER}").as_bytes()), None, Some(TAG)),
            Precondition::Failed
        );
        // …nor trip If-None-Match / the read path into a match.
        assert_eq!(
            evaluate(None, Some(format!("*, {OTHER}").as_bytes()), Some(TAG)),
            Precondition::Proceed
        );
        assert_eq!(
            evaluate_read(Some(format!("*, {OTHER}").as_bytes()), None, TAG, None),
            ReadPrecondition::Proceed
        );
    }
}
