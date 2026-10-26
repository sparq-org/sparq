#![cfg(feature = "wasm")]

//! Mutation witness for the wasm dependency boundary.

use std::process::Command;

#[test]
fn wasm_graph_contains_router_but_no_native_transport_or_crypto() {
    assert_boundary("wasm");
    assert_boundary("wasm,crypt");
}

fn assert_boundary(features: &str) {
    let output = Command::new(env!("CARGO"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "tree",
            "--package",
            "sparq-lws-core",
            "--no-default-features",
            "--features",
            features,
            "--target",
            "wasm32-unknown-unknown",
            "--edges",
            "normal",
        ])
        .output()
        .expect("cargo tree must run for the wasm dependency boundary");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let tree = String::from_utf8(output.stdout).expect("cargo tree output must be UTF-8");
    assert!(
        tree.lines().any(|line| line.contains("axum v")),
        "the portable router dependency is missing from the wasm graph:\n{tree}"
    );

    for forbidden in [
        "async-lock",
        "aws-lc-rs",
        "axum-server",
        "form_urlencoded",
        "hyper-util",
        "mimalloc",
        "object_store",
        "redis v",
        "rustls v",
        "solid-oidc-verifier",
        "spargebra",
        "sparq-lws-crypt",
        "sparq-core",
        "sparq-engine",
        "tokio v",
    ] {
        assert!(
            !tree.lines().any(|line| line.contains(forbidden)),
            "native dependency {forbidden:?} leaked into the wasm graph:\n{tree}"
        );
    }
}
