#!/usr/bin/env python3
"""[GPT-6] Serialized, bounded build/test jobs on the disposable ARM host."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tarfile
import time

BASE = Path("/var/tmp/sparq-build")
RESULTS = Path("/var/tmp/sparq-pod-study")
GIB = 1024 ** 3


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    temporary = Path(str(path) + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def validate_job(job):
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", job["id"]):
        raise ValueError("invalid job id")
    for key, length in (("source_commit", 40), ("archive_sha256", 64)):
        if not re.fullmatch(r"[0-9a-f]{" + str(length) + "}", job[key]):
            raise ValueError("invalid source identity")
    if type(job["timeout_seconds"]) is not int or not 1 <= job["timeout_seconds"] <= 12600:
        raise ValueError("job timeout must be bounded by 12600 seconds")
    commands = job["commands"]
    if not isinstance(commands, list) or not 1 <= len(commands) <= 20:
        raise ValueError("job needs explicit bounded command list")
    for argv in commands:
        if not isinstance(argv, list) or len(argv) < 2 or not all(isinstance(a, str) for a in argv):
            raise ValueError("commands must be argv arrays")
        cargo = argv[0] == "cargo" and argv[1] in {"build", "test", "check", "clippy", "doc"} and "--locked" in argv
        python = argv[0] == "python3" and re.fullmatch(r"(?:[A-Za-z0-9_-]+/)*test[-_][A-Za-z0-9_-]+\.py", argv[1])
        if not (cargo or python) or any("\x00" in a for a in argv):
            raise ValueError("only locked cargo build/test/check/clippy/doc or named Python tests are accepted")


def stop_process(process):
    # A test may fork and exit while descendants remain. End the process group
    # even when its original leader has already exited.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    if process.poll() is None:
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def run_job(job, deadline):
    validate_job(job)
    archive = BASE / "inbox" / (job["id"] + ".tar.gz")
    source = BASE / "sources" / job["id"]
    try:
        return execute_job(job, deadline, archive, source)
    finally:
        # The queue is serialized, and validation restricts this owned basename.
        # Failed checks/extraction must not accumulate source copies on the host.
        if source.exists():
            shutil.rmtree(source)
        archive.unlink(missing_ok=True)


def execute_job(job, deadline, archive, source):
    name = job["id"]
    result = dict(job, scope="build-and-functional-tests-only", started_epoch=time.time(), commands_run=[])
    if shutil.disk_usage(BASE).free < 20 * GIB:
        raise ValueError("disk admission below 20 GiB")
    if digest(archive) != job["archive_sha256"]:
        raise ValueError("source archive checksum mismatch")
    source.mkdir(parents=True, exist_ok=False)
    if archive.stat().st_size > 512 * 1024 ** 2:
        raise ValueError("source archive exceeds build-only bound")
    with tarfile.open(archive) as bundle:
        if sum(member.size for member in bundle.getmembers()) > 2 * GIB:
            raise ValueError("unpacked source exceeds build-only bound")
        bundle.extractall(source, filter="data")
    for required in ("Cargo.lock", "rust-toolchain.toml"):
        result[required + "_sha256"] = digest(source / required)
    limit = min(time.monotonic() + job["timeout_seconds"], time.monotonic() + deadline - time.time() - 900)
    if limit <= time.monotonic():
        raise ValueError("host deadline has insufficient time")
    environment = dict(os.environ, CARGO_TARGET_DIR=str(BASE / "target"), CARGO_BUILD_JOBS="8", RUSTDOCFLAGS="-D warnings")
    result["build_environment"] = {key: environment[key] for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "RUSTDOCFLAGS")}
    failure = None
    for index, argv in enumerate(job["commands"]):
        log = RESULTS / f"{name}-{index}.log"
        with log.open("wb") as stream:
            process = subprocess.Popen(argv, cwd=source, env=environment, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                while process.poll() is None:
                    free = shutil.disk_usage(BASE).free
                    print(json.dumps(dict(event="build-heartbeat", job=name, argv=argv, free_bytes=free)), flush=True)
                    if free < 10 * GIB:
                        failure = "disk-floor-10GiB"
                        break
                    if time.monotonic() >= limit:
                        failure = "bounded-timeout"
                        break
                    time.sleep(min(45, max(.1, limit - time.monotonic())))
            finally:
                stop_process(process)
        result["commands_run"].append(dict(argv=argv, returncode=process.returncode, log=log.name, sha256=digest(log)))
        if failure or process.returncode:
            failure = failure or "command-failed"
            break
    result.update(finished_epoch=time.time(), status=failure or "passed")
    write_json(RESULTS / (name + ".json"), result)
    return result


def finalize(marker):
    (RESULTS / "finished-at.txt").write_text(time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()) + "\n")
    rows = [f"{digest(p)}  {p.name}\n" for p in sorted(RESULTS.iterdir())
            if p.is_file() and p.name not in {"MANIFEST.sha256", "DONE", "FAILED", "READY"}]
    temporary = RESULTS / "MANIFEST.tmp"
    temporary.write_text("".join(rows))
    temporary.replace(RESULTS / "MANIFEST.sha256")
    (RESULTS / "READY").unlink(missing_ok=True)
    (RESULTS / marker).touch()


def serve(deadline):
    RESULTS.mkdir(parents=True, exist_ok=True)
    (BASE / "inbox").mkdir(parents=True, exist_ok=True)
    (BASE / "sources").mkdir(exist_ok=True)
    (RESULTS / "stage.txt").write_text("build-host-setup\n")
    write_json(RESULTS / "runner-scope.json", dict(scope="build-and-functional-tests-only", benchmark=False, deadline_epoch=deadline))
    with (RESULTS / "environment.txt").open("w") as stream:
        for argv in (["uname", "-a"], ["lscpu"], ["df", "-B1", "/var/tmp"], ["rustc", "--version"], ["cargo", "--version"]):
            subprocess.run(argv, stdout=stream, stderr=subprocess.STDOUT, check=True, timeout=900)
    (RESULTS / "source-commit.txt").write_text(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True))
    (RESULTS / "source-bundle.sha256").write_text(digest("/var/tmp/sparq.bundle") + "\n")
    (RESULTS / "stage.txt").write_text("ready-for-explicit-build-jobs\n")
    (RESULTS / "READY").touch()
    seen = set()
    while time.time() < deadline - 600:
        pending = sorted(p for p in (BASE / "inbox").glob("*.json") if p.name not in seen)
        if not pending and (BASE / "FINISH").exists():
            finalize("DONE")
            return
        for path in pending:
            seen.add(path.name)
            job = None
            try:
                job = json.loads(path.read_text())
                if path.name != job["id"] + ".json":
                    raise ValueError("job filename differs from identity")
                run_job(job, deadline)
            except Exception as error:
                write_json(RESULTS / (path.stem + ".json"), dict(status="rejected-or-runner-error", error=str(error), submitted_job=job))
        print("build-host ready; no campaign is running", flush=True)
        time.sleep(45)
    (RESULTS / "failure-detail.txt").write_text("Host deadline reached; explicit FINISH was not received.\n")
    finalize("FAILED")


def connection(host):
    return ["-i", host["key_path"], "-o", "StrictHostKeyChecking=yes", "-o", "UserKnownHostsFile=" + host["known_hosts"], "-o", "ConnectTimeout=15"]


def submit(args):
    host = json.loads(args.host.read_text())
    if time.time() >= host["deadline_epoch"] - 600:
        raise ValueError("host is closing")
    source = args.source.resolve()
    if subprocess.check_output(["git", "status", "--porcelain"], cwd=source):
        raise ValueError("source snapshot must be committed and clean")
    job = dict(id=args.id, source_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip(), timeout_seconds=args.timeout, commands=json.loads(args.commands.read_text()), archive_sha256="0" * 64)
    validate_job(job)
    staging = args.host.parent / "submitted" / args.id
    staging.mkdir(parents=True, exist_ok=False)
    archive = staging / (args.id + ".tar.gz")
    subprocess.run(["git", "archive", "--format=tar.gz", "--output=" + str(archive), "HEAD"], cwd=source, check=True)
    job["archive_sha256"] = digest(archive)
    job_file = staging / (args.id + ".json")
    write_json(job_file, job)
    options = connection(host)
    remote = "ubuntu@" + host["ip"]
    subprocess.run(["ssh", *options, remote,
                    "test -f /var/tmp/sparq-pod-study/READY && test ! -e /var/tmp/sparq-build/inbox/" + args.id + ".json"],
                   check=True, timeout=60)
    subprocess.run(["scp", *options, str(archive), remote + ":/var/tmp/sparq-build/inbox/"], check=True, timeout=300)
    subprocess.run(["scp", *options, str(job_file), remote + ":/var/tmp/sparq-build/inbox/" + args.id + ".pending"], check=True, timeout=60)
    subprocess.run(["ssh", *options, remote, "mv /var/tmp/sparq-build/inbox/" + args.id + ".pending /var/tmp/sparq-build/inbox/" + args.id + ".json"], check=True, timeout=60)
    archive.unlink()
    print(json.dumps(job, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="mode", required=True)
    server = commands.add_parser("serve")
    server.add_argument("--deadline", type=int, required=True)
    job = commands.add_parser("submit")
    job.add_argument("--host", type=Path, required=True)
    job.add_argument("--source", type=Path, required=True)
    job.add_argument("--id", required=True)
    job.add_argument("--timeout", type=int, required=True)
    job.add_argument("--commands", type=Path, required=True, help="JSON array of explicit argv arrays")
    finish = commands.add_parser("finish")
    finish.add_argument("--host", type=Path, required=True)
    args = parser.parse_args()
    if args.mode == "serve":
        try:
            serve(args.deadline)
        except Exception as error:
            RESULTS.mkdir(parents=True, exist_ok=True)
            (RESULTS / "failure-detail.txt").write_text(str(error) + "\n")
            finalize("FAILED")
            raise
    elif args.mode == "submit":
        submit(args)
    else:
        host = json.loads(args.host.read_text())
        subprocess.run(["ssh", *connection(host), "ubuntu@" + host["ip"], "touch /var/tmp/sparq-build/FINISH"], check=True, timeout=60)


if __name__ == "__main__":
    main()
