#![allow(dead_code)]
// Temporary directories are unique across sandbox PID namespaces as well as threads.
pub struct Workspace(pub tempfile::TempDir);
impl Workspace {
    pub fn new(package: &sparq_zk_query::codegen::Package) -> Self {
        let root = tempfile::tempdir().unwrap();
        for (name, body) in &package.files {
            let p = root.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        std::os::unix::fs::symlink(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../zk")
                .canonicalize()
                .unwrap(),
            root.path().join("zk"),
        )
        .unwrap();
        Self(root)
    }
}
