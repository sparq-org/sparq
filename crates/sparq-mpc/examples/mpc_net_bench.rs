// sq-tg6b — the DRIVER / coordinator for the MPC network tier (Tier
// 2/3). It binds a loopback listener, spawns N REAL `mpc_party` CHILD PROCESSES
// (each its own OS process), drives one representative cell over the accepted
// connections, and emits the MEASURED wall-clock + bytes-on-wire as a stable JSON
// shape — the network-tier counterpart of the in-process counting tier's MODELLED
// numbers (`examples/mpc_bench_matrix.rs`).
//
// HONESTY (design record §5): the in-process tier emits MODELLED communication; THIS
// emits MEASURED network cost. The two MUST agree on the protocol RESULT (the
// driver runs the in-process reference alongside and asserts equality), which is
// the correctness anchor — a faster-but-wrong networked run is caught. The
// wall-clock here is a REAL multi-process latency; on raw loopback it is LAN-ideal
// (~0 RTT). For the LAN/WAN-SHAPED figure, run under a netem profile applied by
// `scripts/mpc-netem.sh` on a PRIVILEGED host (CAP_NET_ADMIN) or the EC2 bead
// (sq-hoaj) — `tc qdisc` is not available unprivileged here.
//
// Run (the seeded RNG for the in-process reference + the driver's dealer needs
// the insecure-test-rng feature — OFF by default; it only makes the SIMULATION
// reproducible, never a production claim):
//
//   cargo build  -p sparq-mpc --features insecure-test-rng --examples
//   cargo run    -p sparq-mpc --features insecure-test-rng --example mpc_net_bench -- \
//                --query aggregate --parties 3
//   cargo run    -p sparq-mpc --features insecure-test-rng --example mpc_net_bench -- \
//                --query join --parties 5 --scale 4 --json
//   cargo run    -p sparq-mpc --features insecure-test-rng --example mpc_net_bench -- \
//                --query shuffle --parties 5 --scale 8         # sq-bdbv
//   cargo run    -p sparq-mpc --features insecure-test-rng --example mpc_net_bench -- \
//                --query sort --parties 5 --scale 8 --json     # sq-bdbv
//
// The four cells: `aggregate` + `join` are single-round (deal → local op → open);
// `shuffle` (AS-Waksman switches) + `sort` (Batcher compare-exchanges) are
// MULTI-round — one `Swap` per network gate — so their `rounds` count is the
// network's gate count, the per-switch/per-comparison wall-clock the bead asks for.
//
// (Building the example wires the sibling `mpc_party` binary the driver spawns.)

// The driver needs the SEEDED backend (`new_seeded`) + the bench correctness
// references (`check_aggregate`/`check_hidden_join`), which are gated behind
// `insecure-test-rng` (it only makes the SIMULATION reproducible — never a
// production claim). So the whole driver is feature-gated, with a clear no-op
// `main` otherwise, exactly like `examples/mpc_bench_matrix.rs`. A default build
// of the example therefore still compiles + runs (it prints how to enable it).
#[cfg(not(feature = "insecure-test-rng"))]
fn main() {
    eprintln!(
        "mpc_net_bench needs the insecure-test-rng feature (reproducible simulation \
         seed + correctness references). Re-run with:\n  cargo run -p sparq-mpc \
         --features insecure-test-rng --example mpc_net_bench -- --query aggregate --parties 3"
    );
}

#[cfg(feature = "insecure-test-rng")]
use std::io::Write;
#[cfg(feature = "insecure-test-rng")]
use std::net::TcpListener;
#[cfg(feature = "insecure-test-rng")]
use std::path::PathBuf;
#[cfg(feature = "insecure-test-rng")]
use std::process::{Child, Command, Stdio};

#[cfg(feature = "insecure-test-rng")]
use sparq_mpc::bench::{check_aggregate, check_hidden_join, QueryClass};
#[cfg(feature = "insecure-test-rng")]
use sparq_mpc::transport::{Channel, Coordinator, NetworkCell};
#[cfg(feature = "insecure-test-rng")]
use sparq_mpc::ShamirBackend;

#[cfg(feature = "insecure-test-rng")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let query = arg_value(&args, "--query").unwrap_or_else(|| "aggregate".to_string());
    let parties: usize = arg_value(&args, "--parties")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let scale: usize = arg_value(&args, "--scale")
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let json = args.iter().any(|a| a == "--json");

    let query_class = match query.as_str() {
        "aggregate" | "sum" => QueryClass::CumulativeSumAggregate,
        "join" | "hidden-join" => QueryClass::HiddenValueEquiJoin,
        "shuffle" => QueryClass::ObliviousShuffle,
        "sort" => QueryClass::ObliviousSort,
        other => {
            eprintln!("unknown --query '{other}' (use: aggregate | join | shuffle | sort)");
            std::process::exit(2);
        }
    };

    match run(query_class, parties, scale) {
        Ok((cell, in_proc_result)) => {
            // The correctness anchor: the networked result MUST equal the
            // in-process reference. A divergence is a hard failure, never a
            // silently-emitted number.
            if cell.result != in_proc_result {
                eprintln!(
                    "FATAL: networked result {} != in-process reference {} \
                     (the transport corrupted the protocol)",
                    cell.result, in_proc_result
                );
                std::process::exit(1);
            }
            if json {
                println!("{}", cell_json(&cell, in_proc_result));
            } else {
                print_human(&cell, in_proc_result);
            }
        }
        Err(e) => {
            eprintln!("network-tier run failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Bind a loopback listener, spawn `parties` child party processes pointed at it,
/// build the coordinator over the accepted connections, run the cell, and return
/// the measured cell PLUS the in-process reference result for the anchor check.
#[cfg(feature = "insecure-test-rng")]
fn run(
    query_class: QueryClass,
    parties: usize,
    scale: usize,
) -> Result<(NetworkCell, u64), Box<dyn std::error::Error>> {
    // Fixed seed ⇒ reproducible simulation (NEVER a production claim — the
    // insecure-test-rng feature name is the warning). The same seed feeds the
    // in-process reference so both run the identical sharing.
    let seed = 0x5347_5851_4E45_5430u64; // "SGQNET0"
    let backend = ShamirBackend::new_seeded(parties, seed)?;

    // Inputs + the in-process reference result (the correctness anchor).
    let (values, in_proc_result) = match query_class {
        QueryClass::CumulativeSumAggregate => {
            let values: Vec<u64> = (0..parties as u64).map(|i| 1000 * (i + 1)).collect();
            let reference = check_aggregate(&backend, &values)?;
            (values, reference)
        }
        QueryClass::HiddenValueEquiJoin => {
            let reference = check_hidden_join(&backend, scale)? as u64;
            (Vec::new(), reference)
        }
        QueryClass::ObliviousShuffle => {
            // The reference is the multiset-invariant column SUM of the canonical
            // `i*7+1` input (the in-process `check_shuffle` correctness gate asserts
            // the SAME multiset is preserved). sq-bdbv.
            let reference: u64 = (0..scale as u64).map(|i| i * 7 + 1).sum();
            (Vec::new(), reference)
        }
        QueryClass::ObliviousSort => {
            // Reference = multiset-invariant column sum of the canonical reverse-
            // sorted `scale-i` input. sq-bdbv.
            let reference: u64 = (0..scale as u64).map(|i| scale as u64 - i).sum();
            (Vec::new(), reference)
        }
        QueryClass::HiddenBoundedPath => {
            // The hidden bounded property path (sq-py8h.5) has no live-wire driver —
            // its cost is modelled analytically (bench::run_matrix /
            // BoundedPathPlan::comm_counter), not measured over the socket. The CLI
            // never selects it; this arm keeps the match exhaustive.
            return Err(
                "hidden-bounded-path has no networked driver (cost is modelled \
                        analytically via bench::run_matrix, sq-py8h.5)"
                    .into(),
            );
        }
    };

    // Bind loopback; port 0 ⇒ OS-assigned. The parties connect here.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;

    // Spawn N real party PROCESSES (the genuine process-per-party transport, vs
    // the in-crate test's thread-per-party stand-in).
    let party_bin = party_binary_path()?;
    let mut children: Vec<Child> = Vec::with_capacity(parties);
    for _ in 0..parties {
        let child = Command::new(&party_bin)
            .arg(addr.to_string())
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", party_bin.display()))?;
        children.push(child);
    }

    // Accept N connections, build channels.
    let mut channels = Vec::with_capacity(parties);
    for _ in 0..parties {
        let (stream, _peer) = listener.accept()?;
        stream.set_nodelay(true).ok();
        channels.push(Channel::new(stream));
    }

    let mut coord = Coordinator::new(channels, backend.clone())?;
    let mut dealer = backend.dealer();

    use std::time::Instant;
    let start = Instant::now();
    let (result, rounds) = match query_class {
        QueryClass::CumulativeSumAggregate => (coord.aggregate(&mut dealer, &values)?, 2u64),
        QueryClass::HiddenValueEquiJoin => (coord.hidden_join(&mut dealer, scale)?, 2u64),
        QueryClass::ObliviousShuffle => {
            // Same canonical input as the in-process reference; report the multiset-
            // invariant column sum + the per-switch round count. sq-bdbv.
            let col: Vec<u64> = (0..scale as u64).map(|i| i * 7 + 1).collect();
            let (opened, gates) = coord.shuffle(&mut dealer, &col)?;
            (opened.iter().copied().sum(), gates)
        }
        QueryClass::ObliviousSort => {
            let col: Vec<u64> = (0..scale as u64).map(|i| scale as u64 - i).collect();
            let (opened, gates) = coord.sort(&mut dealer, &col)?;
            (opened.iter().copied().sum(), gates)
        }
        QueryClass::HiddenBoundedPath => {
            // Unreachable from the CLI (no string maps to it); the analytic cost is
            // the only modelled path for this class. Keep the match exhaustive.
            // sq-py8h.5
            return Err("hidden-bounded-path has no networked driver".into());
        }
    };
    let wall_clock = start.elapsed();
    let (bytes_sent, bytes_recv) = coord.byte_totals();

    // Reap the party processes (they exit on Done).
    for mut c in children {
        let _ = c.wait();
    }

    let cell = NetworkCell {
        query_class,
        parties,
        threshold: backend.threshold(),
        scale: if query_class == QueryClass::CumulativeSumAggregate {
            values.len()
        } else {
            scale
        },
        result,
        wall_clock,
        bytes_sent,
        bytes_recv,
        rounds,
    };
    Ok((cell, in_proc_result))
}

/// Locate the sibling `mpc_party` example binary next to this driver (cargo lays
/// example binaries side by side under `target/<profile>/examples/`).
#[cfg(feature = "insecure-test-rng")]
fn party_binary_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut dir = std::env::current_exe()?;
    dir.pop(); // drop the driver's filename → the examples dir
    let bin = dir.join(if cfg!(windows) {
        "mpc_party.exe"
    } else {
        "mpc_party"
    });
    if !bin.exists() {
        return Err(format!(
            "party binary not found at {} — build it with: \
             cargo build -p sparq-mpc --features insecure-test-rng --examples",
            bin.display()
        )
        .into());
    }
    Ok(bin)
}

#[cfg(feature = "insecure-test-rng")]
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

#[cfg(feature = "insecure-test-rng")]
fn cell_json(cell: &NetworkCell, reference: u64) -> String {
    // Stable, dependency-free JSON (mirrors bench::MatrixResults::to_json). The
    // tier label is explicit so a reader never mistakes a measured network number
    // for a modelled one — and the field/profile note states the privilege gate.
    format!(
        "{{\n  \"tier\": \"multi-process-network\",\n  \
         \"note\": \"REAL multi-process loopback transport; wall-clock MEASURED. \
         LAN/WAN shaping needs tc/netem (CAP_NET_ADMIN) — see scripts/mpc-netem.sh \
         + the EC2 bead sq-hoaj\",\n  \
         \"query_class\": \"{}\",\n  \"parties\": {},\n  \"threshold\": {},\n  \
         \"scale\": {},\n  \"result\": {},\n  \"in_process_reference\": {},\n  \
         \"result_matches_in_process\": {},\n  \"wall_clock_micros\": {},\n  \
         \"bytes_sent\": {},\n  \"bytes_recv\": {},\n  \"total_bytes\": {},\n  \
         \"rounds\": {}\n}}",
        cell.query_class.id(),
        cell.parties,
        cell.threshold,
        cell.scale,
        cell.result,
        reference,
        cell.result == reference,
        cell.wall_clock.as_micros(),
        cell.bytes_sent,
        cell.bytes_recv,
        cell.total_bytes(),
        cell.rounds,
    )
}

#[cfg(feature = "insecure-test-rng")]
fn print_human(cell: &NetworkCell, reference: u64) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(
        out,
        "TIER 2 (multi-process, loopback) — REAL transport; wall-clock MEASURED.\n\
         LAN/WAN shaping (Tier 3) needs tc/netem (CAP_NET_ADMIN): scripts/mpc-netem.sh \
         on a privileged host or the EC2 bead sq-hoaj.\n"
    );
    let _ = writeln!(
        out,
        "  query_class : {}\n  parties (N) : {}  (t={})\n  scale       : {}\n  \
         result      : {}  (in-process reference: {}  match: {})\n  \
         wall-clock  : {} µs\n  bytes sent  : {}\n  bytes recv  : {}\n  \
         total bytes : {}\n  rounds      : {}",
        cell.query_class.id(),
        cell.parties,
        cell.threshold,
        cell.scale,
        cell.result,
        reference,
        cell.result == reference,
        cell.wall_clock.as_micros(),
        cell.bytes_sent,
        cell.bytes_recv,
        cell.total_bytes(),
        cell.rounds,
    );
}
