//! Peer-exchange primitives (research §6.2) and the membership-epoch /
//! causal-stability frontier rule (research §4.3).
//!
//! Three pieces:
//!
//! * [`SyncHello`] — the handshake document peers exchange first (dataset id,
//!   protocol format version, membership epoch, causal summary), with the
//!   same strict canonical codec discipline as every other wire document;
//! * [`missing_intervals`] — given what this side has and a remote summary,
//!   the compressed `(origin, first..=last)` sequence intervals the remote is
//!   missing, computed arithmetically from clock + cloud (never by iterating
//!   a clock range, so a huge counter cannot be used as a resource attack);
//! * [`Membership`] + [`StabilityTracker`] — per-epoch acknowledgement
//!   tracking whose [`StabilityTracker::stable_frontier`] is the greatest
//!   summary acknowledged by **every** member that can still write in the
//!   current epoch. Context may be garbage-collected only below that
//!   frontier; a wall-clock retention period is never sufficient
//!   (research §4.3). The computation is deliberately conservative: it takes
//!   the pointwise minimum of the members' clocks and ignores cloud dots, so
//!   it can under-approximate but never over-approximate stability.

use crate::CrdtError;
use crate::codec::{expect_object, expect_str, parse_dec_u64, parse_summary, write_json_string, write_summary};
use crate::envelope::{Admission, Limits};
use crate::id::{DatasetId, Dot, ReplicaId};
use crate::summary::CausalSummary;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The version-1 hello format tag.
pub const HELLO_FORMAT_V1: &str = "sparq-crdt-hello/1";

/// Upper bound on hello document bytes (a hello is one summary plus headers).
const MAX_HELLO_BYTES: usize = 1 << 20;

/// The peer handshake: dataset identity, membership epoch, and this side's
/// causal summary (research §6.2 step 1). The `summary` may be either
/// namespace — a **journal frontier** (for envelope interval exchange) or a
/// **data context** — as long as both sides agree which; the interval
/// primitives here exchange journal frontiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncHello {
    /// The dataset the sender replicates.
    pub dataset: DatasetId,
    /// The sender's membership epoch.
    pub epoch: u64,
    /// The sender's causal summary.
    pub summary: CausalSummary,
}

impl SyncHello {
    /// Convenience constructor.
    pub fn new(dataset: DatasetId, epoch: u64, summary: CausalSummary) -> Self {
        SyncHello { dataset, epoch, summary }
    }

    /// Encodes the canonical byte form
    /// `{"dataset":…,"epoch":…,"format":"sparq-crdt-hello/1","summary":…}`.
    /// Infallible: every reachable value is well-formed by construction.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str("{\"dataset\":");
        write_json_string(&mut out, self.dataset.as_str());
        out.push_str(",\"epoch\":");
        write_json_string(&mut out, &self.epoch.to_string());
        out.push_str(",\"format\":");
        write_json_string(&mut out, HELLO_FORMAT_V1);
        out.push_str(",\"summary\":");
        write_summary(&mut out, &self.summary);
        out.push('}');
        out.into_bytes()
    }

    /// Strictly decodes a hello for `admission` under `limits`, with the same
    /// rejection discipline as the envelope codec: oversized, unknown format
    /// version, wrong dataset, wrong membership epoch, unnormalised summary,
    /// or any non-canonical byte form.
    pub fn decode(
        bytes: &[u8],
        admission: &Admission,
        limits: &Limits,
    ) -> Result<Self, CrdtError> {
        if bytes.len() > MAX_HELLO_BYTES {
            return Err(CrdtError::Oversized {
                what: "hello bytes",
                len: bytes.len(),
                max: MAX_HELLO_BYTES,
            });
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|e| CrdtError::Invalid {
            what: "hello",
            reason: format!("not valid JSON: {e}"),
        })?;
        let map = expect_object(&value, "hello", &["dataset", "epoch", "format", "summary"])?;
        let format = expect_str(map, "hello", "format")?;
        if format != HELLO_FORMAT_V1 {
            return Err(CrdtError::UnsupportedFormat { found: format.to_owned() });
        }
        let dataset = DatasetId::new(expect_str(map, "hello", "dataset")?)?;
        let epoch = parse_dec_u64(expect_str(map, "hello", "epoch")?)?;
        admission.check(&dataset, epoch)?;
        let summary = parse_summary(
            map.get("summary").expect("key checked above"),
            "hello summary",
            limits.max_clock_entries,
            limits.max_cloud_dots,
        )?;
        let hello = SyncHello::new(dataset, epoch, summary);
        if hello.encode() != bytes {
            return Err(CrdtError::NonCanonical {
                reason: "hello bytes are not the canonical encoding of their content".into(),
            });
        }
        Ok(hello)
    }
}

/// A contiguous inclusive run of one origin's sequence numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceInterval {
    /// The origin replica.
    pub origin: ReplicaId,
    /// First missing sequence (inclusive, ≥ 1).
    pub first: u64,
    /// Last missing sequence (inclusive, ≥ `first`).
    pub last: u64,
}

/// Computes the identities denoted by `have` that `remote` is missing, as
/// compressed per-origin intervals in (origin bytes, sequence) order — the
/// "request missing delta intervals" step of the peer protocol (research
/// §6.2 step 2).
///
/// Cost is proportional to the number of clock entries and cloud dots of the
/// two summaries, never to counter magnitudes.
pub fn missing_intervals(
    have: &CausalSummary,
    remote: &CausalSummary,
) -> Vec<SequenceInterval> {
    let mut out: Vec<SequenceInterval> = Vec::new();
    let mut push = |origin: &ReplicaId, first: u64, last: u64| {
        if let Some(prev) = out.last_mut() {
            // `checked_add` so a preceding interval ending at u64::MAX does not
            // overflow the successor test (it simply cannot be adjacent).
            if &prev.origin == origin && prev.last.checked_add(1) == Some(first) {
                prev.last = last;
                return;
            }
        }
        out.push(SequenceInterval { origin: origin.clone(), first, last });
    };
    // Every origin either side of `have` mentions, in raw-byte order (clock
    // and cloud are both BTree-ordered, so a sorted merge falls out of
    // iterating origins from the clock plus the cloud's distinct replicas).
    let mut origins: BTreeSet<&ReplicaId> = have.clock().keys().collect();
    origins.extend(have.cloud().iter().map(Dot::replica));
    for origin in origins {
        let have_clock = have.clock().get(origin).copied().unwrap_or(0);
        let remote_clock = remote.clock().get(origin).copied().unwrap_or(0);
        // Contiguous part: (remote_clock, have_clock] minus the remote cloud.
        if have_clock > remote_clock {
            // `cursor` is the next unaccounted sequence; `None` once it would
            // exceed u64::MAX (exhausted — no successor exists, so nothing more
            // can be missing for this origin). `remote_clock < have_clock <=
            // u64::MAX` here, so the initial `+ 1` cannot overflow.
            let mut cursor: Option<u64> = Some(remote_clock + 1);
            for dot in remote.cloud() {
                if dot.replica() != origin {
                    continue;
                }
                let Some(cur) = cursor else { break };
                let counter = dot.counter();
                if counter < cur || counter > have_clock {
                    continue;
                }
                if counter > cur {
                    push(origin, cur, counter - 1);
                }
                // Advance past this remote-known dot; a dot at u64::MAX has no
                // successor, so the origin is exhausted (skip the trailing
                // interval rather than wrapping to an invalid `first == 0`).
                cursor = counter.checked_add(1);
            }
            if let Some(cur) = cursor {
                if cur <= have_clock {
                    push(origin, cur, have_clock);
                }
            }
        }
        // Sparse part: this side's cloud dots for the origin.
        for dot in have.cloud() {
            if dot.replica() == origin && !remote.contains(dot) {
                push(origin, dot.counter(), dot.counter());
            }
        }
    }
    out
}

/// One membership epoch's writer set (research §4.3). Establishing, agreeing
/// on, and authenticating the membership set is an epoch/coordination
/// protocol **outside** this crate; this type only records its outcome so
/// stability can be tracked against it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Membership {
    epoch: u64,
    members: BTreeSet<ReplicaId>,
}

impl Membership {
    /// Builds a membership record; the member set must be non-empty.
    pub fn new(epoch: u64, members: BTreeSet<ReplicaId>) -> Result<Self, CrdtError> {
        if members.is_empty() {
            return Err(CrdtError::Invalid {
                what: "membership",
                reason: "member set must be non-empty".into(),
            });
        }
        Ok(Membership { epoch, members })
    }

    /// The epoch number.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The replicas that can still write in this epoch.
    pub fn members(&self) -> &BTreeSet<ReplicaId> {
        &self.members
    }
}

/// Tracks per-member causal-summary acknowledgements within one membership
/// epoch and derives the causal-stability frontier: what **every** current
/// member has acknowledged. Garbage collection below that frontier is safe
/// under the membership rule; with any member yet to acknowledge, the
/// frontier is empty and nothing may be collected — the conservative
/// retain-forever default of research §4.3.
#[derive(Clone, Debug)]
pub struct StabilityTracker {
    membership: Membership,
    acks: BTreeMap<ReplicaId, CausalSummary>,
}

impl StabilityTracker {
    /// Starts tracking for one membership epoch with no acknowledgements.
    pub fn new(membership: Membership) -> Self {
        StabilityTracker { membership, acks: BTreeMap::new() }
    }

    /// The membership this tracker is scoped to.
    pub fn membership(&self) -> &Membership {
        &self.membership
    }

    /// Records (or advances) one member's acknowledged summary. Rejects an
    /// acknowledgement from a non-member or for a different epoch, and never
    /// lets an acknowledgement move backwards (acknowledgements are unioned,
    /// so replayed or reordered acks are harmless).
    pub fn record_ack(
        &mut self,
        member: &ReplicaId,
        epoch: u64,
        summary: CausalSummary,
    ) -> Result<(), CrdtError> {
        if epoch != self.membership.epoch {
            return Err(CrdtError::WrongEpoch {
                expected: self.membership.epoch,
                found: epoch,
            });
        }
        if !self.membership.members.contains(member) {
            return Err(CrdtError::Invalid {
                what: "stability ack",
                reason: "acknowledging replica is not a member of the current epoch".into(),
            });
        }
        self.acks
            .entry(member.clone())
            .and_modify(|known| known.union(&summary))
            .or_insert(summary);
        Ok(())
    }

    /// The causal-stability frontier: the pointwise minimum of every current
    /// member's acknowledged clock. Empty until **all** members have
    /// acknowledged. Cloud dots are deliberately ignored (a dot at or below
    /// every member's clock is contained in every member's summary), so the
    /// result can under-approximate but never over-approximate what is
    /// stable.
    pub fn stable_frontier(&self) -> CausalSummary {
        let mut clock: BTreeMap<ReplicaId, u64> = BTreeMap::new();
        for (i, member) in self.membership.members.iter().enumerate() {
            let Some(ack) = self.acks.get(member) else {
                return CausalSummary::new();
            };
            if i == 0 {
                clock = ack.clock().clone();
            } else {
                clock.retain(|origin, n| {
                    let theirs = ack.clock().get(origin).copied().unwrap_or(0);
                    *n = (*n).min(theirs);
                    *n > 0
                });
            }
        }
        CausalSummary::from_parts(clock, Vec::new())
            .expect("a cloud-free clock of positive entries is always normalised")
    }

    /// Advances to a **later** membership epoch (research §4.3: revoke,
    /// re-establish the member set, restart stability). All acknowledgements
    /// are discarded — stability must be re-earned by the new member set.
    pub fn advance_epoch(&mut self, membership: Membership) -> Result<(), CrdtError> {
        if membership.epoch <= self.membership.epoch {
            return Err(CrdtError::WrongEpoch {
                // Diagnostic only; `saturating_add` so a u64::MAX current epoch
                // cannot overflow this successor in the error path.
                expected: self.membership.epoch.saturating_add(1),
                found: membership.epoch,
            });
        }
        self.membership = membership;
        self.acks.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(bytes: &[u8]) -> ReplicaId {
        ReplicaId::new(bytes.to_vec()).expect("valid replica id")
    }

    fn dot(r: &[u8], c: u64) -> Dot {
        Dot::new(rid(r), c).expect("valid dot")
    }

    fn dataset() -> DatasetId {
        DatasetId::new("https://example.test/datasets/team").unwrap()
    }

    fn admission() -> Admission {
        Admission::new(dataset(), 3)
    }

    fn summary(dots: &[(&[u8], u64)]) -> CausalSummary {
        let mut s = CausalSummary::new();
        for (r, c) in dots {
            s.insert(dot(r, *c));
        }
        s
    }

    #[test]
    fn hello_new_and_encode_are_canonical() {
        let hello = SyncHello::new(dataset(), 3, summary(&[(b"peer-a", 1)]));
        assert_eq!(
            String::from_utf8(hello.encode()).unwrap(),
            "{\"dataset\":\"https://example.test/datasets/team\",\"epoch\":\"3\",\
             \"format\":\"sparq-crdt-hello/1\",\
             \"summary\":{\"clock\":{\"cGVlci1h\":\"1\"},\"cloud\":[]}}"
        );
    }

    #[test]
    fn hello_decode_round_trips() {
        let hello = SyncHello::new(dataset(), 3, summary(&[(b"peer-a", 1), (b"peer-b", 5)]));
        let decoded =
            SyncHello::decode(&hello.encode(), &admission(), &Limits::default()).unwrap();
        assert_eq!(decoded, hello);
    }

    #[test]
    fn hello_decode_rejects_wrong_dataset_epoch_version_and_noncanonical() {
        let bytes = SyncHello::new(dataset(), 3, summary(&[])).encode();
        let other = Admission::new(DatasetId::new("https://example.test/other").unwrap(), 3);
        assert!(matches!(
            SyncHello::decode(&bytes, &other, &Limits::default()),
            Err(CrdtError::WrongDataset { .. })
        ));
        let stale = Admission::new(dataset(), 2);
        assert!(matches!(
            SyncHello::decode(&bytes, &stale, &Limits::default()),
            Err(CrdtError::WrongEpoch { .. })
        ));
        let text = String::from_utf8(bytes.clone()).unwrap();
        let unknown = text.replace("sparq-crdt-hello/1", "sparq-crdt-hello/2");
        assert!(matches!(
            SyncHello::decode(unknown.as_bytes(), &admission(), &Limits::default()),
            Err(CrdtError::UnsupportedFormat { .. })
        ));
        let padded = format!("{text} ");
        assert!(SyncHello::decode(padded.as_bytes(), &admission(), &Limits::default()).is_err());
    }

    #[test]
    fn missing_intervals_compresses_the_contiguous_gap() {
        let have = summary(&[(b"a", 1), (b"a", 2), (b"a", 3), (b"a", 4)]);
        let remote = summary(&[(b"a", 1)]);
        assert_eq!(
            missing_intervals(&have, &remote),
            vec![SequenceInterval { origin: rid(b"a"), first: 2, last: 4 }]
        );
        // The remote missing nothing yields no intervals.
        assert!(missing_intervals(&have, &have).is_empty());
        // An empty local summary has nothing to offer.
        assert!(missing_intervals(&summary(&[]), &remote).is_empty());
    }

    #[test]
    fn missing_intervals_splits_around_remote_cloud_dots() {
        let have = summary(&[(b"a", 1), (b"a", 2), (b"a", 3), (b"a", 4), (b"a", 5)]);
        // Remote has clock a:1 plus sparse a:3 — missing 2, 4, 5.
        let remote = summary(&[(b"a", 1), (b"a", 3)]);
        assert_eq!(
            missing_intervals(&have, &remote),
            vec![
                SequenceInterval { origin: rid(b"a"), first: 2, last: 2 },
                SequenceInterval { origin: rid(b"a"), first: 4, last: 5 },
            ]
        );
    }

    #[test]
    fn missing_intervals_covers_local_cloud_dots_and_multiple_origins() {
        // Local: a complete to 2 plus sparse a:5; b sparse b:2 only.
        let have = summary(&[(b"a", 1), (b"a", 2), (b"a", 5), (b"b", 2)]);
        let remote = summary(&[(b"a", 1)]);
        assert_eq!(
            missing_intervals(&have, &remote),
            vec![
                SequenceInterval { origin: rid(b"a"), first: 2, last: 2 },
                SequenceInterval { origin: rid(b"a"), first: 5, last: 5 },
                SequenceInterval { origin: rid(b"b"), first: 2, last: 2 },
            ]
        );
        // Adjacent contiguous + sparse runs merge into one interval.
        let have = summary(&[(b"a", 1), (b"a", 2), (b"a", 3)]);
        let mut remote_none = CausalSummary::new();
        remote_none.insert(dot(b"z", 1));
        assert_eq!(
            missing_intervals(&have, &remote_none),
            vec![SequenceInterval { origin: rid(b"a"), first: 1, last: 3 }]
        );
    }

    #[test]
    fn missing_intervals_handles_u64_max_boundaries_without_overflow() {
        // A local clock at u64::MAX with the remote one short by one: the
        // single missing sequence is exactly u64::MAX, with no successor
        // arithmetic tripping over the boundary.
        let mut have_clock = BTreeMap::new();
        have_clock.insert(rid(b"a"), u64::MAX);
        let have = CausalSummary::from_parts(have_clock, Vec::new()).unwrap();
        let mut remote_clock = BTreeMap::new();
        remote_clock.insert(rid(b"a"), u64::MAX - 1);
        let remote = CausalSummary::from_parts(remote_clock, Vec::new()).unwrap();
        assert_eq!(
            missing_intervals(&have, &remote),
            vec![SequenceInterval { origin: rid(b"a"), first: u64::MAX, last: u64::MAX }]
        );

        // A remote cloud dot exactly at u64::MAX is the last sequence the
        // remote already holds: the cursor advances past it and exhausts,
        // emitting only the prefix gap and never an invalid `first == 0`.
        let mut have_clock = BTreeMap::new();
        have_clock.insert(rid(b"a"), u64::MAX);
        let have = CausalSummary::from_parts(have_clock, Vec::new()).unwrap();
        // Remote: clock a:1 plus a sparse cloud dot a:u64::MAX.
        let mut remote_clock = BTreeMap::new();
        remote_clock.insert(rid(b"a"), 1);
        let remote =
            CausalSummary::from_parts(remote_clock, vec![dot(b"a", u64::MAX)]).unwrap();
        assert_eq!(
            missing_intervals(&have, &remote),
            vec![SequenceInterval { origin: rid(b"a"), first: 2, last: u64::MAX - 1 }]
        );

        // A local cloud dot at u64::MAX the remote lacks is offered as a
        // single-sequence interval — the sparse path also stays overflow-free.
        let mut have_clock = BTreeMap::new();
        have_clock.insert(rid(b"a"), 1);
        let have =
            CausalSummary::from_parts(have_clock, vec![dot(b"a", u64::MAX)]).unwrap();
        let mut remote_clock = BTreeMap::new();
        remote_clock.insert(rid(b"a"), 1);
        let remote = CausalSummary::from_parts(remote_clock, Vec::new()).unwrap();
        assert_eq!(
            missing_intervals(&have, &remote),
            vec![SequenceInterval { origin: rid(b"a"), first: u64::MAX, last: u64::MAX }]
        );
    }

    #[test]
    fn advance_epoch_at_u64_max_reports_saturated_successor_without_overflow() {
        let members: BTreeSet<ReplicaId> = [rid(b"a")].into_iter().collect();
        let mut tracker =
            StabilityTracker::new(Membership::new(u64::MAX, members.clone()).unwrap());
        // No later epoch exists, so this always hits the error path; the
        // diagnostic successor must saturate rather than overflow.
        assert!(matches!(
            tracker.advance_epoch(Membership::new(u64::MAX, members).unwrap()),
            Err(CrdtError::WrongEpoch { expected: u64::MAX, found: u64::MAX })
        ));
    }

    #[test]
    fn membership_new_rejects_an_empty_member_set() {
        assert!(Membership::new(1, BTreeSet::new()).is_err());
        let members: BTreeSet<ReplicaId> = [rid(b"a")].into_iter().collect();
        let membership = Membership::new(1, members.clone()).unwrap();
        assert_eq!(membership.epoch(), 1);
        assert_eq!(membership.members(), &members);
    }

    fn two_member_tracker() -> StabilityTracker {
        let members: BTreeSet<ReplicaId> = [rid(b"a"), rid(b"b")].into_iter().collect();
        StabilityTracker::new(Membership::new(3, members).unwrap())
    }

    #[test]
    fn stability_tracker_new_and_membership_accessor() {
        let tracker = two_member_tracker();
        assert_eq!(tracker.membership().epoch(), 3);
        assert!(tracker.stable_frontier().is_empty());
    }

    #[test]
    fn record_ack_rejects_wrong_epoch_and_non_members() {
        let mut tracker = two_member_tracker();
        assert!(matches!(
            tracker.record_ack(&rid(b"a"), 2, summary(&[])),
            Err(CrdtError::WrongEpoch { .. })
        ));
        assert!(tracker.record_ack(&rid(b"z"), 3, summary(&[])).is_err());
        assert!(tracker.record_ack(&rid(b"a"), 3, summary(&[(b"a", 1)])).is_ok());
    }

    #[test]
    fn stable_frontier_is_the_pointwise_minimum_of_all_members() {
        let mut tracker = two_member_tracker();
        tracker
            .record_ack(&rid(b"a"), 3, summary(&[(b"a", 1), (b"a", 2), (b"b", 1)]))
            .unwrap();
        // Only one of two members acknowledged: nothing is stable yet.
        assert!(tracker.stable_frontier().is_empty());
        tracker.record_ack(&rid(b"b"), 3, summary(&[(b"a", 1), (b"c", 4)])).unwrap();
        let frontier = tracker.stable_frontier();
        assert_eq!(frontier.clock().get(&rid(b"a")), Some(&1)); // min(2, 1)
        assert_eq!(frontier.clock().get(&rid(b"b")), None); // min(1, 0) drops out
        assert_eq!(frontier.clock().get(&rid(b"c")), None);
        assert!(frontier.cloud().is_empty());
    }

    #[test]
    fn record_ack_unions_so_replayed_acks_never_move_backwards() {
        let mut tracker = two_member_tracker();
        tracker.record_ack(&rid(b"a"), 3, summary(&[(b"a", 1), (b"a", 2)])).unwrap();
        tracker.record_ack(&rid(b"b"), 3, summary(&[(b"a", 1), (b"a", 2)])).unwrap();
        assert_eq!(tracker.stable_frontier().clock().get(&rid(b"a")), Some(&2));
        // A stale replayed ack must not regress the member's known summary.
        tracker.record_ack(&rid(b"a"), 3, summary(&[(b"a", 1)])).unwrap();
        assert_eq!(tracker.stable_frontier().clock().get(&rid(b"a")), Some(&2));
    }

    #[test]
    fn advance_epoch_requires_a_later_epoch_and_resets_acks() {
        let mut tracker = two_member_tracker();
        tracker.record_ack(&rid(b"a"), 3, summary(&[(b"a", 1)])).unwrap();
        tracker.record_ack(&rid(b"b"), 3, summary(&[(b"a", 1)])).unwrap();
        assert!(!tracker.stable_frontier().is_empty());
        // Same or earlier epoch is rejected.
        let members: BTreeSet<ReplicaId> = [rid(b"a")].into_iter().collect();
        assert!(tracker.advance_epoch(Membership::new(3, members.clone()).unwrap()).is_err());
        // A later epoch resets stability: it must be re-earned.
        tracker.advance_epoch(Membership::new(4, members).unwrap()).unwrap();
        assert_eq!(tracker.membership().epoch(), 4);
        assert!(tracker.stable_frontier().is_empty());
        assert!(matches!(
            tracker.record_ack(&rid(b"a"), 3, summary(&[])),
            Err(CrdtError::WrongEpoch { .. })
        ));
        tracker.record_ack(&rid(b"a"), 4, summary(&[(b"a", 1)])).unwrap();
        assert_eq!(tracker.stable_frontier().clock().get(&rid(b"a")), Some(&1));
    }
}
