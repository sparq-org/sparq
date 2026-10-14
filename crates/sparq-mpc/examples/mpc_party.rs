// sq-tg6b — the PARTY child binary for the MPC network tier (Tier 2/3).
// One invocation = one MPC compute party, its OWN OS process. It connects to the
// coordinator's loopback address, runs the protocol step(s) the coordinator
// drives, and exits on `Done`. It holds ONLY its own share-points — never the
// cleartext inputs — exactly as a real MPC party would.
//
// Spawned by `examples/mpc_net_bench.rs` (the driver / coordinator); not run by
// hand normally. Usage:
//
//   mpc_party <coordinator_addr>
//
// e.g. `mpc_party 127.0.0.1:54321`. Under Tier 3 the loopback is netem-shaped by
// `scripts/mpc-netem.sh` (privileged) BEFORE the driver starts the parties; the
// party code is identical — only the kernel queue discipline differs.

use std::net::TcpStream;

use sparq_mpc::transport::{run_party, Channel};

fn main() {
    let addr = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: mpc_party <coordinator_addr>  (e.g. 127.0.0.1:54321)");
        std::process::exit(2);
    });

    // Connect to the coordinator. Retry briefly: the driver may spawn the party
    // a hair before its listener is accepting (a normal startup race).
    let stream = connect_with_retry(&addr);
    stream.set_nodelay(true).ok();

    let mut chan = Channel::new(stream);
    match run_party(&mut chan) {
        Ok(()) => {
            // Report this party's measured on-wire bytes to stderr (the driver
            // reads the AUTHORITATIVE coordinator-side byte totals; this is a
            // cross-check trace, not the reported figure).
            eprintln!(
                "party done: sent={} recv={} bytes",
                chan.bytes_sent, chan.bytes_recv
            );
        }
        Err(e) => {
            eprintln!("party protocol error: {e}");
            std::process::exit(1);
        }
    }
}

fn connect_with_retry(addr: &str) -> TcpStream {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match TcpStream::connect(addr) {
            Ok(s) => return s,
            Err(e) => {
                if Instant::now() >= deadline {
                    eprintln!("party could not connect to {addr}: {e}");
                    std::process::exit(1);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}
