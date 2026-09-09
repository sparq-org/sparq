# Complete existing build_with_retry and run_leg production bodies.
def build_with_retry(crate, features):
    # [OPUS-4.8] sq-hhxc heritage: bounded retry absorbs transient crates.io
    # hiccups during any residual resolution the build performs.
    for attempt in range(1, BUILD_ATTEMPTS + 1):
        rc = run([CARGO, "build", "-p", crate, "--features", features])
        if rc == 0:
            return 0
        if attempt < BUILD_ATTEMPTS:
            print(
                f"cargo build attempt {attempt} failed; retrying...",
                file=sys.stderr,
                flush=True,
            )
    return rc


def run_leg(leg):
    """Build -> (test) -> clippy, exactly the per-leg matrix step set.
    Returns None on success, else the name of the failing step."""
    crate, features = leg["crate"], leg["features"]
    if build_with_retry(crate, features) != 0:
        return "build"
    if leg["test"]:
        if run([CARGO, "test", "-p", crate, "--features", features]) != 0:
            return "test"
    if (
        run(
            [
                CARGO,
                "clippy",
                "-p",
                crate,
                "--features",
                features,
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]
        )
        != 0
    ):
        return "clippy"
    return None

