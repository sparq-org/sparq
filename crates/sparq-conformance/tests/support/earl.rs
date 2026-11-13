//! Shared, test-only EARL outcome sink. Unset SPARQ_EARL_OUT is a no-op.

use std::io::Write;

/// Hermetic fixtures for exercising a reporting runner in its own process.
pub struct Fixture(std::path::PathBuf);

impl Fixture {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sparq-earl-report-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    pub fn root(&self) -> &std::path::Path {
        &self.0
    }

    pub fn assert_report(
        &self,
        runner: &str,
        marker: &str,
        roots: &[(&str, &std::path::Path)],
        expected: &[(String, bool)],
    ) {
        let path = self.0.join("outcomes.tsv");
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", runner, "--nocapture"])
            .env("SPARQ_EARL_OUT", &path);
        for (key, root) in roots {
            command.env(key, root);
        }
        let output = command.output().unwrap();
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(
            log.lines().any(|line| line == marker),
            "runner must finish before its ratchet fails: {log}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "tiny fixtures must trip the real ratchet"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let mut got: Vec<_> = text.lines().map(str::to_owned).collect();
        let mut want: Vec<_> = expected
            .iter()
            .map(|(id, passed)| format!("{id}\t{}", if *passed { "passed" } else { "failed" }))
            .collect();
        got.sort();
        want.sort();
        assert_eq!(got, want, "every test needs exactly one truthful outcome");
        assert!(text.ends_with('\n'), "outcome file must end with LF");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn emit<'a>(outcomes: impl IntoIterator<Item = (&'a str, bool)>) {
    let Some(path) = std::env::var_os("SPARQ_EARL_OUT") else {
        return;
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("open SPARQ_EARL_OUT {}: {e}", path.to_string_lossy()));
    write_outcomes(&mut file, outcomes).expect("write SPARQ_EARL_OUT");
}

fn write_outcomes<'a>(
    writer: &mut impl Write,
    outcomes: impl IntoIterator<Item = (&'a str, bool)>,
) -> std::io::Result<()> {
    // One append per batch keeps independently running test functions' lines together.
    let mut text = String::new();
    for (id, passed) in outcomes {
        use std::fmt::Write;
        writeln!(text, "{id}\t{}", if passed { "passed" } else { "failed" }).unwrap();
    }
    writer.write_all(text.as_bytes())
}

#[test]
fn outcome_lines_preserve_iris_and_failures() {
    let mut out = Vec::new();
    write_outcomes(
        &mut out,
        [
            ("https://example.test/manifest#ok", true),
            ("https://example.test/manifest#bad", false),
        ],
    )
    .unwrap();
    assert_eq!(
        out,
        b"https://example.test/manifest#ok\tpassed\nhttps://example.test/manifest#bad\tfailed\n"
    );
}

#[test]
fn outcome_sink_is_opt_in_and_appends() {
    const CHILD: &str = "SPARQ_EARL_SINK_CHILD";
    if std::env::var_os(CHILD).is_some() {
        emit([("https://example.test/manifest#ok", true)]);
        emit([("https://example.test/manifest#bad", false)]);
        return;
    }
    let path = std::env::temp_dir().join(format!("sparq-earl-sink-{}.tsv", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let child = || {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "earl::outcome_sink_is_opt_in_and_appends"])
            .env(CHILD, "1")
            .env_remove("SPARQ_EARL_OUT");
        command
    };
    let output = child().output().unwrap();
    assert!(output.status.success(), "sink child must run");
    assert!(!path.exists(), "unset sink must not create an output file");
    std::fs::write(&path, "https://example.test/manifest#seed\tpassed\n").unwrap();
    let output = child().env("SPARQ_EARL_OUT", &path).output().unwrap();
    assert!(output.status.success(), "sink child must write");
    let got = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(got, "https://example.test/manifest#seed\tpassed\nhttps://example.test/manifest#ok\tpassed\nhttps://example.test/manifest#bad\tfailed\n");
}
