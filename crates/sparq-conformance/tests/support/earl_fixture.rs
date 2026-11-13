//! Optional self-exec fixture helper for the existing EARL reporting tests.

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
        self.assert_report_status(runner, marker, roots, expected, false);
    }

    pub fn assert_report_status(
        &self,
        runner: &str,
        marker: &str,
        roots: &[(&str, &std::path::Path)],
        expected: &[(String, bool)],
        success: bool,
    ) {
        let path = self.0.join("outcomes.tsv");
        let _ = std::fs::remove_file(&path);
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
        assert_eq!(
            output.status.success(),
            success,
            "runner status must reflect the real ratchet: {log}"
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
