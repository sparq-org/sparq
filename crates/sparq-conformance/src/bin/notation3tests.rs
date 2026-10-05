//! sparq-notation3tests: runs sparq's N3 reasoner over the community
//! notation3tests suite (<https://codeberg.org/phochste/notation3tests>, issue
//! #6467) and writes a pass/fail report. The suite is fetched, never vendored:
//!
//! ```text
//! git clone https://codeberg.org/phochste/notation3tests tests/notation3tests
//! cargo run --release -p sparq-conformance --bin sparq-notation3tests -- \
//!     --suite tests/notation3tests --report n3tests-report.md --json n3tests.json
//! ```
//!
//! Each test runs in a CHILD process (this binary re-invoked with `--one`) so
//! a non-terminating document is killed at `--timeout` instead of leaking a
//! spinning thread. Informational: always exits 0 unless `--min-pass N` is
//! given and fewer than N tests pass.
#![forbid(unsafe_code)]

use sparq_conformance::notation3tests::{
    discover, report_json, report_markdown, run_one, Row, Verdict,
};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn usage() -> ! {
    eprintln!(
        "usage: sparq-notation3tests --suite DIR [--report FILE] [--json FILE] [--rev REV] \
         [--timeout SECS] [--filter SUBSTR] [--min-pass N]"
    );
    std::process::exit(2);
}

fn main() {
    let mut suite: Option<PathBuf> = None;
    let mut report: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;
    let mut rev = String::from("unknown");
    let mut timeout = Duration::from_secs(20);
    let mut filter: Option<String> = None;
    let mut min_pass: Option<usize> = None;
    let mut one: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--suite" => suite = Some(PathBuf::from(val())),
            "--report" => report = Some(PathBuf::from(val())),
            "--json" => json = Some(PathBuf::from(val())),
            "--rev" => rev = val(),
            "--timeout" => timeout = Duration::from_secs(val().parse().unwrap_or_else(|_| usage())),
            "--filter" => filter = Some(val()),
            "--min-pass" => min_pass = Some(val().parse().unwrap_or_else(|_| usage())),
            "--one" => one = Some(PathBuf::from(val())),
            _ => usage(),
        }
    }
    let Some(suite) = suite else { usage() };

    // Child mode: one test, verdict line on stdout.
    if let Some(file) = one {
        std::panic::set_hook(Box::new(|_| {}));
        let v = std::panic::catch_unwind(|| run_one(&file, &suite))
            .unwrap_or_else(|_| Verdict::Error("reasoner panicked".into()));
        println!("{}", v.to_line());
        return;
    }

    if !suite.is_dir() {
        eprintln!("suite directory {} not found", suite.display());
        std::process::exit(2);
    }
    let exe = std::env::current_exe().expect("current_exe");
    let tests: Vec<PathBuf> = discover(&suite)
        .into_iter()
        .filter(|p| {
            filter
                .as_deref()
                .is_none_or(|f| p.to_string_lossy().contains(f))
        })
        .collect();
    let mut rows = Vec::with_capacity(tests.len());
    for t in &tests {
        let verdict = run_child(&exe, &suite, t, timeout);
        let name = t.strip_prefix(&suite).unwrap_or(t).display().to_string();
        eprintln!("{} {name}", verdict.to_line());
        rows.push(Row {
            test: name,
            verdict,
        });
    }
    let pass = rows.iter().filter(|r| r.verdict.is_pass()).count();
    println!("notation3tests: {pass} / {} pass", rows.len());
    if let Some(p) = report {
        std::fs::write(&p, report_markdown(&rows, &rev)).expect("write report");
    }
    if let Some(p) = json {
        std::fs::write(&p, report_json(&rows, &rev)).expect("write json");
    }
    if min_pass.is_some_and(|m| pass < m) {
        eprintln!(
            "pass count {pass} below --min-pass {}",
            min_pass.unwrap_or(0)
        );
        std::process::exit(1);
    }
}

fn run_child(exe: &Path, suite: &Path, test: &Path, timeout: Duration) -> Verdict {
    let mut child = match Command::new(exe)
        .arg("--suite")
        .arg(suite)
        .arg("--one")
        .arg(test)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Verdict::Error(format!("spawn: {e}")),
    };
    // Drain stdout concurrently: the reasoner may log to stdout (e.g. log:trace,
    // issue #6466), and a full pipe would block the child into a false timeout.
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Verdict::Timeout;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Verdict::Error(format!("wait: {e}")),
        }
    }
    // The verdict is the child's LAST stdout line (anything before it is reasoner output).
    let out = reader.join().unwrap_or_default();
    let text = String::from_utf8_lossy(&out);
    match text.lines().last() {
        Some(line) => Verdict::from_line(line),
        None => Verdict::Error("child produced no verdict (crashed?)".into()),
    }
}
