#!/usr/bin/env python3
"""Compare two local authorization server binaries on macOS without tracing privileges.

Pass an accepting trust request as --fixture. The stateless arm removes its trust
block. Use the same binary for both arms for an identical-arm control. Servers run
only on loopback; every response must allow and match across revisions. Output
includes raw interleaved pairs and paired bootstrap intervals. No build runs here.
"""
import argparse
import ctypes
import http.client
import json
import os
from pathlib import Path
import random
import socket
import statistics
import subprocess
import time


class TaskInfo(ctypes.Structure):
    # sys/proc_info.h, PROC_PIDTASKINFO; counters cover all threads in the task.
    _fields_ = [(n, ctypes.c_uint64) for n in (
        "virtual_size", "resident_size", "total_user", "total_system",
        "threads_user", "threads_system",
    )] + [(n, ctypes.c_int32) for n in (
        "policy", "faults", "pageins", "cow_faults", "messages_sent", "messages_received",
        "syscalls_mach", "syscalls_unix", "csw", "threadnum", "numrunning", "priority",
    )]


def counters(pid):
    lib = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    lib.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64,
                                ctypes.c_void_p, ctypes.c_int]
    info = TaskInfo()
    size = ctypes.sizeof(info)
    if lib.proc_pidinfo(pid, 4, 0, ctypes.byref(info), size) != size:
        raise OSError(ctypes.get_errno(), "proc_pidinfo(PROC_PIDTASKINFO) failed")
    return {"unix": info.syscalls_unix, "mach": info.syscalls_mach,
            "system_ns": info.total_system, "context_switches": info.csw}


def launch(binary, workers):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env = dict(os.environ, RAYON_NUM_THREADS=str(workers), TOKIO_WORKER_THREADS="2",
               SPARQ_SOLID_AUTHZ="1", SPARQ_SOLID_AUTHZ_TRUST="1")
    process = subprocess.Popen([str(Path(binary).resolve()), "--addr", f"127.0.0.1:{port}"],
                               env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(200):
            if process.poll() is not None:
                raise RuntimeError(f"server exited: {process.returncode}")
            try:
                conn = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
                conn.connect()
                return process, conn
            except ConnectionRefusedError:
                time.sleep(.05)
        raise TimeoutError("server did not start")
    except BaseException:
        process.terminate()
        process.wait()
        raise


def request(conn, body):
    conn.request("POST", "/authz/decide", body, {"Content-Type": "application/json"})
    response = conn.getresponse()
    payload = json.loads(response.read())
    if response.status != 200 or payload.get("allow") is not True:
        raise AssertionError((response.status, payload))
    return payload


def summarize(pairs, key):
    a = [p[0][key] for p in pairs]
    b = [p[1][key] for p in pairs]
    rng = random.Random(41)
    ratios = []
    for _ in range(10000):
        indices = rng.choices(range(len(pairs)), k=len(pairs))
        den = sum(a[i] for i in indices)
        ratios.append(sum(b[i] for i in indices) / den if den else 1.)
    ratios.sort()
    return {"base": statistics.mean(a), "head": statistics.mean(b),
            "ratio": sum(b) / sum(a) if sum(a) else 1.,
            "ci95": [ratios[250], ratios[9749]]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--fixture", required=True)
    parser.add_argument("--pairs", type=int, default=40)
    parser.add_argument("--batch", type=int, default=200)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    if args.pairs < 1 or args.batch < 1:
        parser.error("--pairs and --batch must be positive")
    fixture = json.loads(Path(args.fixture).read_text())
    stateless = dict(fixture)
    del stateless["trust"]
    result = {}
    for workers in [1, 4]:
        servers = []
        try:
            servers.append(launch(args.base, workers))
            servers.append(launch(args.head, workers))
            for lane, value in [("stateless", stateless), ("trust", fixture)]:
                body = json.dumps(value).encode()
                reference = request(servers[0][1], body)
                if lane == "trust":
                    assert reference["trustJustification"]["admittedCount"] > 0
                for _, conn in servers:
                    for _ in range(100):
                        assert request(conn, body) == reference
                pairs = []
                for pair in range(args.pairs):
                    samples = [None, None]
                    for arm in ([0, 1] if pair % 2 == 0 else [1, 0]):
                        process, conn = servers[arm]
                        before = counters(process.pid)
                        start = time.perf_counter_ns()
                        for _ in range(args.batch):
                            assert request(conn, body) == reference
                        elapsed = time.perf_counter_ns() - start
                        after = counters(process.pid)
                        samples[arm] = {k: (after[k] - before[k]) / args.batch for k in before}
                        samples[arm]["wall_ns"] = elapsed / args.batch
                        samples[arm]["syscalls"] = samples[arm]["unix"] + samples[arm]["mach"]
                    pairs.append(samples)
                name = f"{lane}/rayon-{workers}"
                result[name] = {"samples": pairs, "summary": {
                    key: summarize(pairs, key) for key in samples[0]
                }}
                print(name, json.dumps(result[name]["summary"]), flush=True)
        finally:
            for process, conn in servers:
                conn.close()
                process.terminate()
                process.wait()
    Path(args.output).write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
