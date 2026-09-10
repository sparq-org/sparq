from pathlib import Path
import os, subprocess, time, signal, json, hashlib, shutil, difflib, re, sys

p = Path(__file__).resolve().parent
r = p.parent.parent
w = r / "worktrees/issue6483"
started = time.monotonic()
commands = []
resources = []
sha = lambda b: hashlib.sha256(b).hexdigest()
initial_worktree_bytes = json.loads((p / "initial-resource.json").read_text())["worktree_allocated_bytes"]

def put(name, value):
    (p / name).write_text(json.dumps(value, indent=2) + "\n" if not isinstance(value, str) else value)

def check(initial=False):
    allocated = initial_worktree_bytes + sum(f.stat().st_blocks * 512 for f in p.rglob("*") if f.is_file() and not f.is_symlink())
    row = {"elapsed_seconds": time.monotonic() - started, "allocated_including_worktree": allocated, "free_bytes": shutil.disk_usage(p).free}
    resources.append(row)
    put("resources.json", resources)
    assert row["free_bytes"] >= (2214592512 if initial else 2147483648), "free floor"
    assert allocated < 125829120, "128MiB cap less8MiB export reserve"
    assert row["elapsed_seconds"] < 300, "total300s"
    return row

def run(name, argv, limit, expected=0, cwd=None):
    check()
    start = time.monotonic()
    stopped = None
    with (p / (name + ".stdout")).open("wb") as out, (p / (name + ".stderr")).open("wb") as err:
        proc = subprocess.Popen(argv, cwd=cwd or q, env=env, stdout=out, stderr=err, start_new_session=True)
        while proc.poll() is None:
            try:
                check()
                assert time.monotonic() - start < limit, "per-command limit"
            except Exception as exc:
                stopped = str(exc)
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()
                break
            time.sleep(0.25)
        code = proc.wait()
    row = {"name": name, "argv": argv, "exit": code, "expected_exit": expected, "seconds": time.monotonic() - start, "stopped": stopped}
    commands.append(row)
    put("commands.json", commands)
    print(json.dumps(row), flush=True)
    check()
    assert stopped is None and code == expected, "unexpected command result; stop"

try:
    check(True)
    ready = json.loads((r / "direct-6483/readiness/artifact-source-readiness.json").read_text())
    compiler = Path(ready["compiler"]["path"])
    assert sha(compiler.read_bytes()) == ready["compiler"]["sha256"]
    for item in ready["externs"]:
        assert sha(Path(item["path"]).read_bytes()) == item["expected_sha256"]
    source = (w / "crates/sparq-bench/src/update_fuzz.rs").read_text()
    assert subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=w, text=True).strip() == "f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464"
    names = re.findall(r"#\[test\]\s+fn (\w+)", source)
    assert len(names) == 26 and sum(n.startswith("raw_duplicate_") for n in names) == 6
    put("selected-tests.json", names)
    guard_start = source.index("        for (label, lines) in [(label_a, a), (label_b, b)] {")
    guard_end = source.index("        return Verdict::Same;", guard_start)
    guard = source[guard_start:guard_end]
    control = source[:guard_start] + source[guard_end:]
    put("guard-removal.diff", "".join(difflib.unified_diff(source.splitlines(True), control.splitlines(True), fromfile="candidate/update_fuzz.rs", tofile="control/update_fuzz.rs")))
    put("removed-guard.rs.txt", guard)
    q = p / "harness/crates/sparq-bench"
    q.mkdir(parents=True)
    (p / "harness/bench").mkdir()
    allow = (w / "bench/differential-divergences.json").read_bytes()
    assert allow == subprocess.check_output(["git", "show", "f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464:bench/differential-divergences.json"], cwd=w)
    (p / "harness/bench/differential-divergences.json").write_bytes(allow)
    assert (q / "../../bench/differential-divergences.json").resolve().read_bytes() == allow
    (p / "tmp").mkdir()
    controlled = json.loads((r / "direct-5183/review-followup/commands.json").read_text())[1]["environment"]
    controlled.update(TMPDIR=str(p / "tmp"), CARGO_MANIFEST_DIR=str(q))
    env = {k: v for k, v in os.environ.items() if not k.startswith(("SPARQ_", "CARGO_PROFILE_")) and k not in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS")}
    env.update(controlled)
    put("provenance.json", {"compiler": ready["compiler"], "externs": ready["externs"], "source_sha256": sha(source.encode()), "allowlist_sha256": sha(allow), "environment": controlled, "edition": "2021", "profile": "O3/unwind/no-LTO/codegen16", "dependency_builds": 0})
    old_argv = json.loads((r / "direct-5183/copilot-duplicate-witness/old-compiler-argv.json").read_text())
    argv_by_name = {}
    for name, text in [("candidate", source), ("control", control)]:
        dest = p / name
        dest.mkdir()
        (dest / "out").mkdir()
        (dest / "update_fuzz.rs").write_text(text)
        (dest / "driver.rs").write_text('// [GPT-6 Astra] Exact module diagnostic driver; the test harness controls execution.\n#[path = "' + str(dest / "update_fuzz.rs") + '"]\nmod update_fuzz;\nfn main() { update_fuzz::run(4141222487, 1); }\n')
        argv = old_argv.copy()
        argv[argv.index("--crate-name") + 1] = "issue6483_" + name
        argv[argv.index("--out-dir") + 1] = str(dest / "out")
        argv[argv.index("src/main.rs")] = str(dest / "driver.rs")
        for i, arg in enumerate(argv):
            if arg.startswith("metadata="):
                argv[i] = "metadata=issue6483_" + name
            if arg.startswith("extra-filename="):
                argv[i] = "extra-filename=-fixed"
        argv_by_name[name] = argv
        put(name + "-compiler-argv.json", argv)
    put("protocol.json", {"before_execution": True, "candidate": "One actual-module build and full26tests: unchanged20plus6new.", "control": "One compiled guard-removal mutant and exact regression filteredrun expected101 plusSame assertionmessage.", "limits": {"seconds": 300, "allocated": 134217728, "reserve": 8388608, "free": 2147483648, "jobs": 1}, "no_dependency_build_or_retry": True})
    run("candidate-build", argv_by_name["candidate"], 100)
    cb = p / "candidate/out/issue6483_candidate-fixed"
    run("candidate-tests", [str(cb), "--test-threads=1", "--nocapture"], 60)
    assert "26 passed; 0 failed" in (p / "candidate-tests.stdout").read_text()
    run("control-build", argv_by_name["control"], 100)
    mb = p / "control/out/issue6483_control-fixed"
    run("control-test", [str(mb), "--exact", "update_fuzz::tests::raw_duplicate_redistribution_is_rejected", "--test-threads=1", "--nocapture"], 20, 101)
    assert "equal-total raw duplicate redistribution returned Same" in (p / "control-test.stderr").read_text()
    clippy = compiler.with_name("clippy-driver")
    assert clippy.is_file(), "missing warm clippy"
    (p / "clippy-out").mkdir()
    argv = argv_by_name["candidate"].copy()
    argv[0] = str(clippy)
    argv[argv.index("--out-dir") + 1] = str(p / "clippy-out")
    argv[argv.index("--emit=dep-info,link")] = "--emit=metadata"
    argv += ["-D", "warnings"]
    run("clippy-tests", argv, 40)
    put("binaries.json", [{"name": name, "sha256": sha((p / name / "out" / ("issue6483_" + name + "-fixed")).read_bytes()), "bytes": (p / name / "out" / ("issue6483_" + name + "-fixed")).stat().st_size} for name in ["candidate", "control"]])
    put("phase-result.json", {"status": "complete", "seconds": time.monotonic() - started, "commands_terminal": len(commands), "pending": False})
except Exception as exc:
    put("phase-result.json", {"status": "stopped", "reason": str(exc), "seconds": time.monotonic() - started, "commands_terminal": len(commands), "pending": False})
    print("STOP", str(exc), flush=True)
    sys.exit(1)
