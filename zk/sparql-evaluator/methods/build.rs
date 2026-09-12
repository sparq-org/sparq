// [GPT-6] Build the pinned guest; unavailable tools are an error, never a mock.
fn main() {
    // Host Clippy injects its own compiler driver; carrying that wrapper into
    // the SDK's cross compilation selects a host sysroot without guest std.
    // Re-exec only this build step with clean child environment, rather than
    // mutating process-global environment or skipping actual guest compilation.
    let wrappers = ["RUSTC_WORKSPACE_WRAPPER", "RUSTC_WRAPPER"];
    if wrappers.iter().any(|key| std::env::var_os(key).is_some()) {
        let mut child =
            std::process::Command::new(std::env::current_exe().expect("build executable"));
        for key in wrappers {
            child.env_remove(key);
        }
        let status = child.status().expect("isolated guest build process");
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        return;
    }
    risc0_build::embed_methods();
}
