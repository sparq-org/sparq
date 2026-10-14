//! Property tests for the windowing-soundness invariant.

use std::collections::BTreeMap;

use proptest::prelude::*;
use sparq_rsp::{Window, WindowSpec, WindowedStream};

fn flatten(windows: &[Window<u16>]) -> BTreeMap<u16, Vec<(u64, u64)>> {
    let mut memberships: BTreeMap<u16, Vec<(u64, u64)>> = BTreeMap::new();
    for window in windows {
        for item in &window.triples {
            memberships
                .entry(item.triple)
                .or_default()
                .push((window.start, window.end));
        }
    }
    memberships
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn count_windows_have_exact_capacity_and_take_closed_drains(
        rows in 1usize..12,
        timestamps in prop::collection::vec(any::<u16>(), 1..80),
    ) {
        let mut stream = WindowedStream::empty(WindowSpec::count(rows).with_slide(rows));
        for (id, ts) in timestamps.iter().enumerate() {
            stream.push(id as u16, u64::from(*ts));
        }

        let closed = stream.take_closed();
        prop_assert!(closed.iter().all(|window| window.triples.len() == rows));
        prop_assert!(stream.take_closed().is_empty());
        // Count-window flush only drains; it must not manufacture a short report.
        prop_assert!(stream.flush().is_empty());
    }

    #[test]
    fn time_windows_match_half_open_membership(
        range in 1u64..20,
        step in 1u64..20,
        increments in prop::collection::vec(0u8..8, 1..60),
    ) {
        let mut stream = WindowedStream::empty(WindowSpec::time(range, step));
        let mut timestamp = 0u64;
        let mut arrivals = Vec::new();
        let mut closed = Vec::new();
        for (id, increment) in increments.into_iter().enumerate() {
            timestamp += u64::from(increment);
            let id = id as u16;
            arrivals.push((id, timestamp));
            stream.push(id, timestamp);
            closed.extend(stream.take_closed());
        }
        closed.extend(stream.flush());

        let actual = flatten(&closed);
        for (id, ts) in arrivals {
            let expected: Vec<_> = closed.iter()
                .filter(|window| window.start <= ts && ts < window.end)
                .map(|window| (window.start, window.end))
                .collect();
            prop_assert_eq!(actual.get(&id).cloned().unwrap_or_default(), expected);
        }
        prop_assert_eq!(stream.late_dropped(), 0);
        prop_assert!(stream.take_closed().is_empty());
    }

    #[test]
    fn watermark_is_monotone_and_advance_is_idempotent(
        range in 1u64..20,
        step in 1u64..20,
        delay in 0u64..20,
        high in 20u64..200,
    ) {
        let mut stream = WindowedStream::<u16>::empty(
            WindowSpec::time(range, step).with_max_delay(delay),
        );
        stream.advance(high);
        let first = stream.take_closed();
        stream.advance(high);
        stream.advance(high.saturating_sub(1));
        prop_assert!(stream.take_closed().is_empty());

        stream.advance(high.saturating_add(range).saturating_add(step));
        let later = stream.take_closed();
        if let (Some(previous), Some(next)) = (first.last(), later.first()) {
            prop_assert!(next.start > previous.start, "an already-closed window was emitted again");
        }
        prop_assert!(stream.take_closed().is_empty());
    }

    #[test]
    fn fully_late_arrival_is_dropped_counted_once_and_never_emitted(
        range in 1u64..30,
        delay in 0u64..30,
        late_ts in 0u64..30,
    ) {
        let late_ts = late_ts % range;
        let horizon = range.saturating_add(delay);
        let mut stream = WindowedStream::empty(
            WindowSpec::time(range, range).with_max_delay(delay),
        );
        stream.push(1u16, horizon);
        let mut closed = stream.take_closed();
        let before = stream.late_dropped();

        stream.push(2u16, late_ts);
        prop_assert_eq!(stream.late_dropped(), before + 1);
        closed.extend(stream.take_closed());
        closed.extend(stream.flush());
        prop_assert!(closed.iter().all(|window| {
            window.triples.iter().all(|item| item.triple != 2)
        }), "late payload appeared in a closed window");
        prop_assert!(stream.take_closed().is_empty());
    }
}
