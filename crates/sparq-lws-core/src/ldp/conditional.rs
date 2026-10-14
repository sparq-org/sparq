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

/// The entity-tag list a precondition header carries across all of its field lines, which combine
/// as one comma-separated list (RFC 9110 §5.3), or `None` when the header is absent. A byte above
/// ASCII (`obs-text`, valid inside a quoted entity tag, §8.8.3) is read as a replacement character,
/// so a tag holding one still parses (and matches none of this server's ASCII tags) while every
/// other tag on its line is kept.
pub(crate) fn tag_header(
    headers: &http::HeaderMap,
    name: http::HeaderName,
) -> Option<std::borrow::Cow<'_, str>> {
    let mut lines = headers
        .get_all(name)
        .iter()
        .map(|v| String::from_utf8_lossy(v.as_bytes()));
    let first = lines.next()?;
    let rest: Vec<_> = lines.collect();
    if rest.is_empty() {
        return Some(first);
    }
    let mut all = first.into_owned();
    for line in rest {
        all.push_str(", ");
        all.push_str(&line);
    }
    Some(std::borrow::Cow::Owned(all))
}

/// Evaluate RFC 9110 §13.2.2 steps 1 and 2 for a GET/HEAD: `If-Match`, or without it
/// `If-Unmodified-Since`, before the read's own [`evaluate_read`]. `If-Match` uses EXACT strong
/// comparison against the selected representation's tag (§8.8.3.2), variant suffix included, so a
/// tag of another negotiated representation of the same state fails (a client must not combine
/// pieces of two representations); `*` passes, the resource existing. `If-Unmodified-Since` fails
/// when `last_modified` is after the date and is ignored without a time (a container listing).
pub fn evaluate_read_guard(
    if_match: Option<&str>,
    if_unmodified_since: Option<&str>,
    current_etag: &str,
    last_modified: Option<SystemTime>,
) -> Precondition {
    if let Some(im) = if_match {
        let ok = is_wildcard(im) || tag_list(im).any(|t| t.matches_strong(current_etag));
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
    if_match: Option<&str>,
    if_unmodified_since: Option<&str>,
) -> Option<i64> {
    if if_match.is_some() {
        return None;
    }
    if_unmodified_since.and_then(|v| crate::http_date::parse(v, crate::clock::now()))
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

    /// STRONG comparison, EXACT opaque tags (for the READ path's `If-Match`): both validators strong
    /// and the opaque tags equal, variant suffix included.
    fn matches_strong(&self, current_strong: &str) -> bool {
        !self.weak && parse_current(current_strong) == Some(self.opaque)
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
