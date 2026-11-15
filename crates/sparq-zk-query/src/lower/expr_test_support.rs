//! Temporary directories acquired atomically across concurrent test processes.
use std::io::ErrorKind;
use std::path::PathBuf;

pub(super) struct Scratch(pub(super) PathBuf);
impl Scratch {
    pub(super) fn new(prefix: &str) -> Self {
        for suffix in 0..u64::MAX {
            let path =
                std::env::temp_dir().join(format!("{prefix}_{}_{suffix}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create expression test directory: {error}"),
            }
        }
        panic!("expression test directory capacity");
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove expression test directory");
    }
}
#[test]
fn simultaneous_directories_are_distinct() {
    let a = Scratch::new("sparq_noir_expr_fixture");
    let b = Scratch::new("sparq_noir_expr_fixture");
    assert_ne!(a.0, b.0);
    assert!(a.0.is_dir() && b.0.is_dir());
}
