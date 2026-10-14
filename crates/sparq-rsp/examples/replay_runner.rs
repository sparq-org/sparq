//! (sq-3f5ay) sparq-side REPLAY-FILE runner — the sparq half of the
//! matched-workload leg of the bounded count-matched-replay RSP protocol
//! (`research/comparative-benchmarking-everything.md` §5.2).
//!
//! `rsp_oracle.rs` replays *in-code* scripts; RSP4J's `Rsp4jReplayRunner.java`
//! replays a pinned `bench/rsp/replay/*.ts.tsv` FILE. Until this runner existed
//! there was no way to drive sparq-rsp from that same file, so the two engines'
//! throughput numbers came from different workloads and the sustained-rate axis
//! stayed **NOT-MEASURED** (`research/gap-rsp-2026-07.md`). This runner closes
//! that gap: it reads the identical pinned replay and emits the SAME comparator
//! contract the Java runner emits, so `bench/rsp/rsp4j_compare.py count-match`
//! can gate per-window counts first and only then publish both engines'
//! sustained triples/s side by side.
//!
//! INVARIANT (unchanged): no throughput row without per-window count agreement.
//! This runner only *reports*; the gate lives in `rsp4j_compare.py`.
//!
//! Output — the comparator contract (`rsp4j_compare.py` `parse_competitor`):
//!
//! ```text
//! meta\t<key>\t<value>                    engine / scenario / mode / policy metadata
//! w<k>\t<rows>                            per-window result-row count, k ascending
//! timing\t<metric>\t<value>\t<unit>       push-loop timing (admitted only past the gate)
//! ```
//!
//! Parsing is done BEFORE the timed section (matching `Rsp4jReplayRunner`, which
//! also parses the whole replay up front), so the timing covers the push +
//! window-evaluation loop only — not TSV I/O.
//!
//! Scenario definitions are kept byte-identical to `rsp_oracle.rs`. That
//! duplication is deliberate and *guarded*: `bench/rsp/run.sh` re-derives the
//! `srbench_join` counts through THIS runner from the pinned replay export and
//! asserts them against `expected.tsv`, so a drift between the two examples
//! fails the per-commit gate loudly.
//!
//! Run:
//! ```sh
//! cargo run --release -p sparq-rsp --example replay_runner -- \
//!   --replay bench/rsp/replay/srbench.ts.tsv --scenario srbench_join
//! ```

use std::time::Instant;

use oxrdf::{Literal, NamedNode, Term};
use sparq_rsp::{ContinuousMultiQuery, ContinuousQuery, EvalMode, WindowResult, WindowSpec};

// ------------------------------------------------------------- replay parsing

/// One replay event: `(ts, stream, subject, predicate, object)`.
struct Event {
    ts: u64,
    stream: NamedNode,
    triple: [Term; 3],
}

/// Minimal N-Triples term reader — the mirror of `Rsp4jReplayRunner.term`:
/// an IRI `<...>` or a typed literal `"lex"^^<datatype>`. The pinned replays use
/// nothing else, and anything else is a hard error rather than a silent guess.
fn parse_term(t: &str) -> Result<Term, String> {
    if let Some(iri) = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        return Ok(NamedNode::new(iri).map_err(|e| format!("bad IRI {t}: {e}"))?.into());
    }
    if let Some(rest) = t.strip_prefix('"') {
        if let Some((lex, dt)) = rest.rsplit_once("\"^^<") {
            let dt = dt
                .strip_suffix('>')
                .ok_or_else(|| format!("unterminated datatype in {t}"))?;
            let dt = NamedNode::new(dt).map_err(|e| format!("bad datatype in {t}: {e}"))?;
            return Ok(Literal::new_typed_literal(lex, dt).into());
        }
    }
    Err(format!("unsupported term syntax: {t}"))
}

fn parse_iri(t: &str) -> Result<NamedNode, String> {
    let iri = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(t);
    NamedNode::new(iri).map_err(|e| format!("bad IRI {t}: {e}"))
}

/// Parses a pinned `.ts.tsv` replay: 5 tab-separated columns, `#` comments,
/// timestamps ascending (asserted — an unsorted file would silently change which
/// window an event lands in, so it is refused rather than sorted for the caller).
fn parse_replay(path: &str) -> Result<Vec<Event>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut events: Vec<Event> = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 5 {
            return Err(format!(
                "{path}:{}: expected 5 tab-separated columns, got {}",
                lineno + 1,
                c.len()
            ));
        }
        let ts: u64 = c[0]
            .parse()
            .map_err(|e| format!("{path}:{}: bad ts {:?}: {e}", lineno + 1, c[0]))?;
        if events.last().is_some_and(|p| p.ts > ts) {
            return Err(format!("{path}:{}: events are not sorted by ts", lineno + 1));
        }
        events.push(Event {
            ts,
            stream: parse_iri(c[1])?,
            triple: [parse_term(c[2])?, parse_term(c[3])?, parse_term(c[4])?],
        });
    }
    if events.is_empty() {
        return Err(format!("{path}: no events"));
    }
    Ok(events)
}

// ----------------------------------------------------------------- scenarios
//
// Byte-identical to the corresponding entries in rsp_oracle.rs (see the module
// header on why the duplication is guarded rather than factored out).

/// Single-window scenario: `(WindowSpec, SPARQL)`; multi-window: RSP-QL text.
enum Query {
    Single(WindowSpec, &'static str),
    Multi(&'static str),
}

fn scenario(name: &str) -> Result<Query, String> {
    Ok(match name {
        "tumbling_avg" => Query::Single(
            WindowSpec::time(10, 10),
            "SELECT (AVG(?v) AS ?avg) WHERE { ?s <http://ex/value> ?v }",
        ),
        "sliding_sum" => Query::Single(
            WindowSpec::time(20, 10),
            "SELECT ?s (SUM(?v) AS ?sum) WHERE { ?s <http://ex/value> ?v } \
             GROUP BY ?s ORDER BY ?s",
        ),
        "tumbling_groupby_join" => Query::Single(
            WindowSpec::time(20, 20),
            "SELECT ?room (AVG(?v) AS ?avg) (COUNT(?v) AS ?n) \
             WHERE { ?s <http://ex/in> ?room . ?s <http://ex/value> ?v } \
             GROUP BY ?room ORDER BY ?room",
        ),
        "srbench_join" => Query::Multi(
            "\
REGISTER STREAM <http://ex/out> AS
SELECT ?st ?state ?v WHERE {
  WINDOW <http://ex/wo> { ?st <http://ex/value> ?v }
  WINDOW <http://ex/wm> { ?st <http://ex/state> ?state }
}
FROM NAMED WINDOW <http://ex/wo> ON <http://ex/obs> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/wm> ON <http://ex/meta> RANGE 10 STEP 10",
        ),
        "srbench_groupby_state" => Query::Multi(
            "\
REGISTER STREAM <http://ex/out> AS
SELECT ?state (COUNT(?v) AS ?n) WHERE {
  WINDOW <http://ex/wo> { ?st <http://ex/value> ?v }
  WINDOW <http://ex/wm> { ?st <http://ex/state> ?state }
}
GROUP BY ?state ORDER BY ?state
FROM NAMED WINDOW <http://ex/wo> ON <http://ex/obs> RANGE 10 STEP 10
FROM NAMED WINDOW <http://ex/wm> ON <http://ex/meta> RANGE 10 STEP 10",
        ),
        other => return Err(format!("unknown scenario {other}")),
    })
}

fn eval_mode(tag: &str) -> Result<EvalMode, String> {
    Ok(match tag {
        "rebuild" => EvalMode::Rebuild,
        "pdict" => EvalMode::PersistentDict,
        "delta" => EvalMode::Delta,
        "snapshot" => EvalMode::Snapshot,
        other => return Err(format!("unknown eval mode {other}")),
    })
}

// -------------------------------------------------------------------- driver

/// Replays `events` through the scenario, returning `(per-window row counts, push
/// wall-nanos)`. Only the push + flush loop is timed.
fn replay(query: Query, mode: EvalMode, events: &[Event]) -> Result<(Vec<usize>, u128), String> {
    let mut counts: Vec<usize> = Vec::new();
    let elapsed = match query {
        Query::Single(spec, sparql) => {
            let mut q = ContinuousQuery::register(sparql, spec)?.with_mode(mode);
            let mut emit = |r: WindowResult| counts.push(r.rows.len());
            let start = Instant::now();
            for e in events {
                q.push(e.triple.clone(), e.ts, &mut emit)?;
            }
            q.flush(&mut emit)?;
            start.elapsed()
        }
        Query::Multi(rspql) => {
            // ContinuousMultiQuery carries the EvalMode of its RSP-QL declaration;
            // `--mode` is reported for the envelope but only steers the
            // single-window path (see the meta line the caller emits).
            let mut q = ContinuousMultiQuery::register(rspql)?;
            let mut emit = |r: WindowResult| counts.push(r.rows.len());
            let start = Instant::now();
            for e in events {
                q.push(&e.stream, e.triple.clone(), e.ts, &mut emit)?;
            }
            q.flush(&mut emit)?;
            start.elapsed()
        }
    };
    Ok((counts, elapsed.as_nanos()))
}

fn run() -> Result<(), String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut replay_path = None;
    let mut scenario_name = None;
    let mut mode_tag = "pdict".to_string();
    let mut i = 0;
    while i + 1 < argv.len() {
        match argv[i].as_str() {
            "--replay" => replay_path = Some(argv[i + 1].clone()),
            "--scenario" => scenario_name = Some(argv[i + 1].clone()),
            "--mode" => mode_tag = argv[i + 1].clone(),
            other => return Err(format!("unknown argument {other}")),
        }
        i += 2;
    }
    if i != argv.len() {
        return Err(format!("dangling argument {}", argv[i]));
    }
    let replay_path = replay_path.ok_or("missing required --replay <file>")?;
    let scenario_name = scenario_name.ok_or("missing required --scenario <name>")?;

    let events = parse_replay(&replay_path)?;
    let query = scenario(&scenario_name)?;
    let mode = eval_mode(&mode_tag)?;
    let (counts, wall_ns) = replay(query, mode, &events)?;

    let mut out = String::new();
    out.push_str("meta\tengine\tsparq-rsp\n");
    out.push_str(&format!("meta\tversion\t{}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("meta\tscenario\t{scenario_name}\n"));
    out.push_str(&format!("meta\tmode\t{mode_tag}\n"));
    out.push_str(&format!("meta\treplay_events\t{}\n", events.len()));
    out.push_str(
        "meta\ttime_model\tclock-free: a window closes on the pushed-timestamp watermark, \
         never the wall clock\n",
    );
    for (k, rows) in counts.iter().enumerate() {
        out.push_str(&format!("w{k}\t{rows}\n"));
    }
    let secs = wall_ns as f64 / 1e9;
    let tps = if secs > 0.0 {
        (events.len() as f64 / secs).round() as u64
    } else {
        0
    };
    out.push_str(&format!("timing\trsp_replay_push_wall_us\t{}\tus\n", wall_ns / 1_000));
    out.push_str(&format!(
        "timing\trsp_replay_push_triples_per_s\t{tps}\ttriples_per_s\n"
    ));
    print!("{out}");
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("[rsp-replay-runner] ERROR: {e}");
        std::process::exit(2);
    }
}
