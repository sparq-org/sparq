//! [OPUS-4.8] sq-2999l (gh-906): OPT-IN HTTP change-data-capture (CDC) poll endpoint in the
//! Amazon-Neptune-Streams [`GetRecords`](https://docs.aws.amazon.com/neptune/latest/userguide/streams-using.html)
//! shape, over the MERGED durable change-stream (sq-b4fns / #1223, in `sparq-serve`).
//!
//! ## What this module is
//!
//! A PURE (no async, no HTTP types) builder — like `crate::tpf` / `crate::descriptors` (both
//! behind their own features, hence not intra-doc-linked) — that
//! (1) parses the `GetRecords` query parameters (`iteratorType` / `at` / `after` / `limit`) into a
//! resolved start offset + bound, and (2) given the polled, ordered slice of
//! [`sparq_serve::ChangeRecord`]s, renders the JSON response body (the ordered change records plus a
//! continuation token). Keeping it here makes the parameter resolution, the per-quad flattening and
//! the continuation maths unit-testable without booting a server. The async handler that wires it
//! onto the axum router (the OPT-IN flag check, the read-auth gate, the disk poll) lives in
//! [`crate::http`] (`streams_endpoint`).
//!
//! ## The poll model (Neptune `GetRecords`, adapted to sparq's per-commit seq)
//!
//! The durable [`ChangeLog`](sparq_serve::ChangeLog) records ONE change record per committed
//! generation, each carrying a monotonic, gapless **sequence number** (`seq`) — the poll offset.
//! A consumer polls from an offset and replays forward, persisting the returned continuation token
//! to resume exactly where it left off (resumable across a process restart — the on-disk log is the
//! source of truth). The start offset comes from the `iteratorType`:
//!
//!   * `TRIM_HORIZON` — from the OLDEST RETAINED record: replay everything the log still holds.
//!   * `AT_SEQUENCE_NUMBER` (`at=N`) — from record `N` inclusive.
//!   * `AFTER_SEQUENCE_NUMBER` (`after=N`) — from record `N+1` (resume after the last one seen).
//!   * `LATEST` — only records committed strictly after this call (start at `next_seq`): a tail.
//!
//! [OPUS-5] (sq-iz7ag) "Oldest retained" is literal: retention (`ChangeLog::apply_retention`,
//! sq-n9s4d) drops whole old segments, moving the **trim horizon**
//! ([`ChangeLog::first_seq`](sparq_serve::ChangeLog::first_seq)) forward off seq 0. So
//! `TRIM_HORIZON` resolves to the horizon, NOT to 0 — a poll below the horizon fails closed in the
//! durable log (it never silently skips dropped records), which would otherwise turn the one
//! iterator type meaning "replay everything retained" into a `500` on any trimmed log. An `at` /
//! `after` anchor resolving below the horizon is the honest
//! [`TrimmedOffset`](crate::streams::TrimmedOffset) rejection instead:
//! the records that consumer asked for are gone, and quietly restarting it at the horizon would
//! skip records it believes it still had coming.
//!
//! `limit` caps the number of **change records (commits)** in one response (a bounded page — the
//! whole point of a poll API); the continuation token (`nextSequenceNumber`) is the seq to pass as
//! `after=` (or `at=`) on the next poll. `hasMoreRecords` says whether the log already holds more
//! beyond this page.
//!
//! ## Response shape
//!
//! Each commit's quad-level changes are flattened to one stream record per `(op, quad)` — the
//! Neptune `records[]` shape — carrying an `eventId` of `{ commitNum: <seq>, opNum: <1-based within
//! the commit> }`, the `op` (`ADD` / `REMOVE`), the affected quad as an N-Quads `stmt`, the
//! `generation` and the commit `timestamp`:
//!
//! ```json
//! { "records": [
//!     { "eventId": { "commitNum": 1, "opNum": 1 }, "op": "ADD",
//!       "generation": 2, "commitTimestampNanos": 1718000000000000000,
//!       "data": { "stmt": "<http://ex/s> <http://ex/p> <http://ex/o> ." } } ],
//!   "totalRecords": 1,
//!   "lastSequenceNumber": 1,
//!   "nextSequenceNumber": 2,
//!   "hasMoreRecords": false }
//! ```
//!
//! [FABLE-5] (gh-2436) An operator **re-base gap record** (`ChangeRecord::rebase`, appended
//! by `POST /admin/change-stream/rebase` to resync a broken stream) renders as ONE marker
//! entry with `"op": "REBASE"` and no `data`: the span before its `generation` was NOT
//! captured, so a consumer needing a complete view re-bootstraps (e.g. from a backup at or
//! after that generation) before replaying on.
//!
//! ## OPT-IN — feature `change-stream` AND a configured log directory, both OFF by default
//!
//! Compiled only behind the `change-stream` cargo feature; even then the route refuses (`404`)
//! unless a durable log directory is configured ([`ServerConfig::change_stream_dir`](crate::ServerConfig::change_stream_dir),
//! CLI `--change-stream <DIR>` / env `SPARQ_CHANGE_STREAM`). This mirrors the `tpf` double-opt-in.
//! It is a **READ-only** poll over the durable log — there is no write path here (records are
//! produced by the commit-recording hook on the update path). At-rest encryption + cryptographic
//! authenticity of the records are out of scope (the same boundary as the backup family).

use sparq_serve::{ChangeOp, ChangeRecord};

/// The default page size — the maximum number of change records (commits) one poll returns when
/// the request does not set `limit`. A bounded page is the whole point of a poll API; 100 mirrors
/// the conventional default of a record-batch poll (and the TPF page default in `crate::tpf`).
pub const DEFAULT_LIMIT: usize = 100;

/// The hard ceiling on `limit` — a request asking for more is clamped to this. Bounds the
/// per-response work + body size regardless of the client-supplied `limit`.
pub const MAX_LIMIT: usize = 10_000;

/// The Neptune-style stream iterator type — where a poll starts reading from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IteratorType {
    /// From the OLDEST RETAINED record (the log's trim horizon — seq 0 until retention drops a
    /// segment): replay everything the log still holds.
    TrimHorizon,
    /// From record `seq` INCLUSIVE (`at=N`).
    AtSequenceNumber(u64),
    /// From the record AFTER `seq` (`after=N` → start at `N + 1`): resume after the last seen.
    AfterSequenceNumber(u64),
    /// Only records committed strictly after this call starts (start at the log's current
    /// `next_seq`): a live tail with no historical replay.
    Latest,
}

/// An error resolving the poll parameters into an [`IteratorType`]. The HTTP layer maps it to a
/// `400` (the message names only the offending parameter, never echoing unbounded caller input).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamError(pub String);

impl std::fmt::Display for ParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// [OPUS-5] (sq-iz7ag) A resolved start offset that retention has already trimmed away — the
/// fail-closed rejection [`IteratorType::from_seq`] returns for an `AT_SEQUENCE_NUMBER` /
/// `AFTER_SEQUENCE_NUMBER` anchor below the log's trim horizon.
///
/// The HTTP layer maps it to a `410 Gone` — the requested records are permanently gone, the same
/// posture (and status) the server already uses for a time-travel `from` outside the retention
/// window. Without it the poll reaches the durable log, which fails it closed as an unreadable
/// offset, and the operator sees an opaque `500` for what is really a caller-recoverable
/// "your checkpoint is older than what we still keep — re-bootstrap" condition.
///
/// Both fields are parsed `u64` sequence numbers — never raw caller text — so rendering them to
/// the client echoes no unbounded caller input (the #241 info-leak posture).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimmedOffset {
    /// The start offset the request resolved to (`at=N` ⇒ `N`; `after=N` ⇒ `N + 1`).
    pub requested: u64,
    /// The trim horizon — the oldest seq the log still retains.
    pub horizon: u64,
}

impl std::fmt::Display for TrimmedOffset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // [OPUS-4.8] positional format args (CodeQL rust/unused-variable false-positive).
        write!(
            f,
            "start offset {} precedes the change-stream trim horizon {} — retention dropped those \
             records; resume at or after {}, or use iteratorType=TRIM_HORIZON to replay from the \
             horizon",
            self.requested, self.horizon, self.horizon
        )
    }
}

impl IteratorType {
    /// Resolves the iterator type from the raw `iteratorType` / `at` / `after` parameters.
    ///
    /// `iterator_type` is case-insensitive; an absent / empty value defaults to `TRIM_HORIZON`
    /// (replay from the start) unless an `after` / `at` parameter is present, in which case the
    /// corresponding sequence-anchored type is inferred (so a bare `?after=5` resumes without the
    /// caller also having to spell out `iteratorType=AFTER_SEQUENCE_NUMBER`). A sequence-anchored
    /// type with a missing / non-numeric anchor is a [`ParamError`] (fail-closed — never a silent
    /// fall back to replaying the whole stream, which would re-deliver records the consumer already
    /// processed).
    pub fn parse(
        iterator_type: Option<&str>,
        at: Option<&str>,
        after: Option<&str>,
    ) -> Result<Self, ParamError> {
        let parse_seq = |raw: Option<&str>, which: &str| -> Result<u64, ParamError> {
            match raw.map(str::trim) {
                Some(s) if !s.is_empty() => s.parse::<u64>().map_err(|_| {
                    // [OPUS-4.8] positional format arg (CodeQL rust/unused-variable false-positive).
                    ParamError(format!(
                        "`{}` must be a non-negative integer sequence number",
                        which
                    ))
                }),
                _ => Err(ParamError(format!(
                    "`{}` requires a sequence number",
                    which
                ))),
            }
        };
        let norm = iterator_type
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_uppercase());
        match norm.as_deref() {
            Some("TRIM_HORIZON") => Ok(IteratorType::TrimHorizon),
            Some("LATEST") => Ok(IteratorType::Latest),
            Some("AT_SEQUENCE_NUMBER") => Ok(IteratorType::AtSequenceNumber(parse_seq(at, "at")?)),
            Some("AFTER_SEQUENCE_NUMBER") => Ok(IteratorType::AfterSequenceNumber(parse_seq(
                after, "after",
            )?)),
            Some(other) => Err(ParamError(format!(
                "unknown iteratorType {:?} (expected TRIM_HORIZON, LATEST, AT_SEQUENCE_NUMBER or \
                 AFTER_SEQUENCE_NUMBER)",
                other
            ))),
            // No explicit iteratorType: infer from an `after` / `at` anchor, else replay from start.
            None if after.map(str::trim).is_some_and(|s| !s.is_empty()) => Ok(
                IteratorType::AfterSequenceNumber(parse_seq(after, "after")?),
            ),
            None if at.map(str::trim).is_some_and(|s| !s.is_empty()) => {
                Ok(IteratorType::AtSequenceNumber(parse_seq(at, "at")?))
            }
            None => Ok(IteratorType::TrimHorizon),
        }
    }

    /// The first sequence number this iterator reads (the `from_seq` to poll the log with), given
    /// the log's current `next_seq` (used only by `LATEST`, which starts at the tail) and its
    /// `trim_horizon` — the oldest seq still retained
    /// ([`ChangeLog::first_seq`](sparq_serve::ChangeLog::first_seq); `0` for a never-trimmed log).
    ///
    /// [OPUS-5] (sq-iz7ag) `TRIM_HORIZON` resolves to `trim_horizon`, not to seq 0: the older
    /// records are gone, and polling the durable log below its horizon fails closed (a `500`) — so
    /// pinning the whole-stream replay at 0 makes it impossible to replay a trimmed log at all.
    /// A sequence-anchored type whose resolved start is below the horizon is a [`TrimmedOffset`]
    /// error rather than a silent bump up to the horizon, which would skip records the consumer
    /// believes it has not received yet.
    pub fn from_seq(self, next_seq: u64, trim_horizon: u64) -> Result<u64, TrimmedOffset> {
        let requested = match self {
            // Everything still retained — by definition at or after the horizon.
            IteratorType::TrimHorizon => return Ok(trim_horizon),
            IteratorType::AtSequenceNumber(n) => n,
            // Saturating so `after=u64::MAX` does not wrap to 0 and replay everything.
            IteratorType::AfterSequenceNumber(n) => n.saturating_add(1),
            // The tail is never behind the horizon: retention never drops the active segment.
            IteratorType::Latest => return Ok(next_seq),
        };
        if requested < trim_horizon {
            return Err(TrimmedOffset {
                requested,
                horizon: trim_horizon,
            });
        }
        Ok(requested)
    }
}

/// Parses + clamps the `limit` parameter: absent / non-numeric → [`DEFAULT_LIMIT`]; `0` is treated
/// as the default (a poll always makes progress); anything above [`MAX_LIMIT`] is clamped down.
pub fn parse_limit(raw: Option<&str>) -> usize {
    match raw.and_then(|s| s.trim().parse::<usize>().ok()) {
        Some(0) | None => DEFAULT_LIMIT,
        Some(n) => n.min(MAX_LIMIT),
    }
}

/// The resolved, bounded result of a poll — the page of records to render plus the continuation
/// state. Computed by [`page`] from the full polled slice; consumed by [`to_json`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<'a> {
    /// The change records in this page (at most `limit`), in order.
    pub records: &'a [ChangeRecord],
    /// `true` when the log already holds records beyond this page (the poll was truncated by
    /// `limit`), so the caller should poll again.
    pub has_more: bool,
    /// The seq the NEXT poll should resume from (pass as `after=last` ⇒ this, or `at=` directly):
    /// one past the last record in this page, or — for an empty page — the requested start offset
    /// (so a tail-poll that found nothing resumes at the same place next time).
    pub next_seq: u64,
}

/// Slices the full polled `records` (every record with `seq >= from_seq`, in order) down to one
/// page of at most `limit` records, computing the continuation token. `from_seq` is the resolved
/// start offset (so an empty page reports it as `next_seq`, not 0).
pub fn page(records: &[ChangeRecord], from_seq: u64, limit: usize) -> Page<'_> {
    let take = records.len().min(limit);
    let slice = &records[..take];
    let has_more = records.len() > take;
    let next_seq = slice
        .last()
        .map(|r| r.seq.saturating_add(1))
        .unwrap_or(from_seq);
    Page {
        records: slice,
        has_more,
        next_seq,
    }
}

/// Renders a [`Page`] to the JSON response body (the Neptune `GetRecords`-shaped document). Each
/// commit's quad-level changes are flattened to one stream record per `(op, quad)` with a
/// `{ commitNum, opNum }` event id. `serde_json` is already a `server`-feature dependency, so the
/// JSON is built with the same escaping discipline the rest of the server's JSON uses.
pub fn to_json(page: &Page<'_>) -> String {
    let mut records = Vec::new();
    for rec in page.records {
        // [FABLE-5] (gh-2436) An operator re-base GAP record (`ChangeRecord::rebase`, no
        // changes — `ChangeLog::rebase_to` / `POST /admin/change-stream/rebase`) must be
        // VISIBLE to a consumer: flattening it to zero entries would silently skip the one
        // record that says "the span before this generation was NOT captured — re-bootstrap
        // before trusting the diff stream". Rendered as ONE marker entry with `"op":
        // "REBASE"` and no `data` (there is no quad).
        if rec.rebase {
            records.push(serde_json::json!({
                "eventId": { "commitNum": rec.seq, "opNum": 1 },
                "op": "REBASE",
                "generation": rec.generation,
                "commitTimestampNanos": rec.timestamp_unix_nanos.to_string(),
            }));
            continue;
        }
        // `opNum` is 1-based WITHIN the commit (Neptune convention), in the record's own
        // deterministic change order (inserts first, then deletes — see `ChangeRecord`).
        for (i, change) in rec.changes.iter().enumerate() {
            let op = match change.op {
                ChangeOp::Insert => "ADD",
                ChangeOp::Delete => "REMOVE",
            };
            records.push(serde_json::json!({
                "eventId": { "commitNum": rec.seq, "opNum": i + 1 },
                "op": op,
                "generation": rec.generation,
                "commitTimestampNanos": rec.timestamp_unix_nanos.to_string(),
                "data": { "stmt": change.quad },
            }));
        }
    }
    let last_seq = page.records.last().map(|r| r.seq);
    let value = serde_json::json!({
        "records": records,
        // The number of COMMITS in this page (a quick header count; the flattened `records` array
        // may be longer — one entry per quad-level change).
        "totalRecords": page.records.len(),
        "lastSequenceNumber": last_seq,
        "nextSequenceNumber": page.next_seq,
        "hasMoreRecords": page.has_more,
    });
    // `to_string` on a serde_json::Value never fails.
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_serve::Change;

    fn rec(seq: u64, generation: u64, changes: Vec<(ChangeOp, &str)>) -> ChangeRecord {
        ChangeRecord {
            seq,
            generation,
            timestamp_unix_nanos: 1_000 + seq as u128,
            changes: changes
                .into_iter()
                .map(|(op, quad)| Change {
                    op,
                    quad: quad.to_string(),
                })
                .collect(),
            rebase: false,
        }
    }

    #[test]
    fn iterator_type_resolves_each_variant() {
        assert_eq!(
            IteratorType::parse(Some("TRIM_HORIZON"), None, None).unwrap(),
            IteratorType::TrimHorizon
        );
        assert_eq!(
            IteratorType::parse(Some("latest"), None, None).unwrap(),
            IteratorType::Latest
        );
        assert_eq!(
            IteratorType::parse(Some("AT_SEQUENCE_NUMBER"), Some("3"), None).unwrap(),
            IteratorType::AtSequenceNumber(3)
        );
        assert_eq!(
            IteratorType::parse(Some("AFTER_SEQUENCE_NUMBER"), None, Some("7")).unwrap(),
            IteratorType::AfterSequenceNumber(7)
        );
    }

    #[test]
    fn bare_after_or_at_is_inferred_without_explicit_iterator_type() {
        assert_eq!(
            IteratorType::parse(None, None, Some("5")).unwrap(),
            IteratorType::AfterSequenceNumber(5)
        );
        assert_eq!(
            IteratorType::parse(None, Some("2"), None).unwrap(),
            IteratorType::AtSequenceNumber(2)
        );
        // Nothing at all → replay from the start.
        assert_eq!(
            IteratorType::parse(None, None, None).unwrap(),
            IteratorType::TrimHorizon
        );
    }

    #[test]
    fn sequence_anchored_without_anchor_is_fail_closed_400() {
        // AT/AFTER with a missing or non-numeric anchor is an error — NEVER a silent replay-all.
        assert!(IteratorType::parse(Some("AT_SEQUENCE_NUMBER"), None, None).is_err());
        assert!(IteratorType::parse(Some("AFTER_SEQUENCE_NUMBER"), None, Some("x")).is_err());
        assert!(IteratorType::parse(Some("nonsense"), None, None).is_err());
    }

    #[test]
    fn from_seq_maps_each_iterator_type() {
        // An untrimmed log: the horizon is 0 and every offset resolves as before.
        assert_eq!(IteratorType::TrimHorizon.from_seq(9, 0), Ok(0));
        assert_eq!(IteratorType::AtSequenceNumber(4).from_seq(9, 0), Ok(4));
        assert_eq!(IteratorType::AfterSequenceNumber(4).from_seq(9, 0), Ok(5));
        assert_eq!(IteratorType::Latest.from_seq(9, 0), Ok(9));
        // `after=u64::MAX` saturates rather than wrapping to 0.
        assert_eq!(
            IteratorType::AfterSequenceNumber(u64::MAX).from_seq(9, 0),
            Ok(u64::MAX)
        );
    }

    /// [OPUS-5] (sq-iz7ag) THE retention regression guard: on a trimmed log `TRIM_HORIZON` must
    /// resolve to the horizon, not to seq 0. Pinning it at 0 sends the poll below the log's
    /// earliest retained record, which the durable log rejects fail-closed — i.e. a `500` on the
    /// one iterator type that means "replay everything retained". Change the `TrimHorizon` arm
    /// back to `Ok(0)` and this goes red.
    #[test]
    fn trim_horizon_resolves_to_the_oldest_retained_record_not_zero() {
        assert_eq!(IteratorType::TrimHorizon.from_seq(9, 4), Ok(4));
        // `LATEST` is still the tail — retention never drops the active segment.
        assert_eq!(IteratorType::Latest.from_seq(9, 4), Ok(9));
    }

    /// [OPUS-5] (sq-iz7ag) An `at` / `after` anchor resolving BELOW the horizon is rejected with
    /// the offsets the HTTP layer turns into a `410` — never silently bumped up to the horizon
    /// (that would skip dropped records the consumer still believes are coming).
    #[test]
    fn anchored_offset_below_the_trim_horizon_is_rejected_with_both_offsets() {
        assert_eq!(
            IteratorType::AtSequenceNumber(1).from_seq(9, 4),
            Err(TrimmedOffset {
                requested: 1,
                horizon: 4
            })
        );
        // `after=2` starts at 3, still below the horizon.
        assert_eq!(
            IteratorType::AfterSequenceNumber(2).from_seq(9, 4),
            Err(TrimmedOffset {
                requested: 3,
                horizon: 4
            })
        );
        // The boundary is the RESOLVED start, not the anchor: `after=3` starts exactly AT the
        // horizon, so nothing was dropped from under that consumer and the poll is fine.
        assert_eq!(IteratorType::AfterSequenceNumber(3).from_seq(9, 4), Ok(4));
        assert_eq!(IteratorType::AtSequenceNumber(4).from_seq(9, 4), Ok(4));
    }

    #[test]
    fn trimmed_offset_message_names_both_offsets_and_the_recovery() {
        let msg = TrimmedOffset {
            requested: 1,
            horizon: 4,
        }
        .to_string();
        assert!(msg.contains("trim horizon"), "{}", msg);
        assert!(msg.contains('1') && msg.contains('4'), "{}", msg);
        assert!(msg.contains("TRIM_HORIZON"), "{}", msg);
    }

    #[test]
    fn limit_parses_clamps_and_defaults() {
        assert_eq!(parse_limit(None), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("abc")), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("0")), DEFAULT_LIMIT);
        assert_eq!(parse_limit(Some("3")), 3);
        assert_eq!(parse_limit(Some("999999")), MAX_LIMIT);
    }

    #[test]
    fn page_truncates_and_reports_continuation() {
        let records = vec![
            rec(2, 3, vec![(ChangeOp::Insert, "a")]),
            rec(3, 4, vec![(ChangeOp::Delete, "b")]),
            rec(4, 5, vec![(ChangeOp::Insert, "c")]),
        ];
        // Page of 2 from a poll that returned 3 → truncated, more available, resume at seq 4.
        let p = page(&records, 2, 2);
        assert_eq!(p.records.len(), 2);
        assert!(p.has_more);
        assert_eq!(p.next_seq, 4);
        assert_eq!(p.records[0].seq, 2);
        assert_eq!(p.records[1].seq, 3);

        // A page big enough for all → no more, resume past the last (seq 5).
        let p = page(&records, 2, 100);
        assert_eq!(p.records.len(), 3);
        assert!(!p.has_more);
        assert_eq!(p.next_seq, 5);
    }

    #[test]
    fn empty_poll_reports_requested_offset_as_next_seq() {
        // A tail poll that found nothing must resume at the SAME offset (not 0), so it does not
        // re-replay the whole stream on the next poll.
        let p = page(&[], 9, 100);
        assert_eq!(p.records.len(), 0);
        assert!(!p.has_more);
        assert_eq!(p.next_seq, 9);
    }

    #[test]
    fn json_flattens_changes_to_one_record_per_quad_with_event_ids() {
        let records = vec![rec(
            1,
            2,
            vec![
                (
                    ChangeOp::Insert,
                    "<http://ex/s> <http://ex/p> <http://ex/o> .",
                ),
                (
                    ChangeOp::Delete,
                    "<http://ex/x> <http://ex/p> <http://ex/y> .",
                ),
            ],
        )];
        let p = page(&records, 1, 100);
        let body = to_json(&p);
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        // One commit, two quad-level changes → two flattened stream records.
        assert_eq!(v["totalRecords"], 1);
        assert_eq!(v["records"].as_array().unwrap().len(), 2);
        assert_eq!(v["lastSequenceNumber"], 1);
        assert_eq!(v["nextSequenceNumber"], 2);
        assert_eq!(v["hasMoreRecords"], false);
        // First change: ADD, opNum 1, commitNum 1, generation 2.
        let r0 = &v["records"][0];
        assert_eq!(r0["op"], "ADD");
        assert_eq!(r0["eventId"]["commitNum"], 1);
        assert_eq!(r0["eventId"]["opNum"], 1);
        assert_eq!(r0["generation"], 2);
        assert_eq!(
            r0["data"]["stmt"],
            "<http://ex/s> <http://ex/p> <http://ex/o> ."
        );
        // Second change: REMOVE, opNum 2.
        assert_eq!(v["records"][1]["op"], "REMOVE");
        assert_eq!(v["records"][1]["eventId"]["opNum"], 2);
        // The timestamp is a string (u128 does not fit a JSON number losslessly).
        assert!(r0["commitTimestampNanos"].is_string());
    }

    /// [FABLE-5] (gh-2436) A re-base gap record (no changes) must NOT flatten to nothing —
    /// it renders as one explicit `"op": "REBASE"` marker entry, so a consumer replaying
    /// past it KNOWS the stream is not a complete diff history there. A regression back to
    /// pure per-change flattening turns this red (the gap would silently disappear).
    #[test]
    fn json_renders_a_rebase_gap_record_as_an_explicit_marker() {
        let gap = ChangeRecord {
            seq: 1,
            generation: 5,
            timestamp_unix_nanos: 1_001,
            changes: Vec::new(),
            rebase: true,
        };
        let records = vec![
            rec(0, 1, vec![(ChangeOp::Insert, "a")]),
            gap,
            rec(2, 6, vec![(ChangeOp::Insert, "b")]),
        ];
        let p = page(&records, 0, 100);
        let body = to_json(&p);
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(v["totalRecords"], 3);
        let flattened = v["records"].as_array().unwrap();
        assert_eq!(flattened.len(), 3, "the gap contributes ONE marker entry");
        let marker = &flattened[1];
        assert_eq!(marker["op"], "REBASE");
        assert_eq!(marker["eventId"]["commitNum"], 1);
        assert_eq!(marker["generation"], 5);
        assert!(
            marker.get("data").is_none(),
            "a gap marker carries no quad: {marker}"
        );
        // The surrounding commits still render as ordinary ADDs.
        assert_eq!(flattened[0]["op"], "ADD");
        assert_eq!(flattened[2]["op"], "ADD");
        assert_eq!(v["nextSequenceNumber"], 3);
    }

    #[test]
    fn json_empty_page_has_null_last_seq_and_empty_records() {
        let p = page(&[], 0, 100);
        let body = to_json(&p);
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(v["totalRecords"], 0);
        assert!(v["records"].as_array().unwrap().is_empty());
        assert!(v["lastSequenceNumber"].is_null());
        assert_eq!(v["nextSequenceNumber"], 0);
    }
}
