#!/usr/bin/env python3
"""Run one QuickSilver proof (Diet Mac'n'Cheese verifier and prover on
localhost) and print one JSON object.

Usage: quicksilver_run.py DIETMC CONFIG IRDIR PORT [--count-bytes]

- prover_ms: prover wall-clock time, from its launch to its exit.
- prover_cpu_ms, verifier_cpu_ms: user + system CPU time of each process.
- verifier_online_ms: verifier wall-clock time from the prover's launch to the
  verifier's exit. The protocol is interactive, so the verifier is online for
  the whole proof; this is the time it is connected, not its work.
- verifier_setup_ms: verifier wall-clock time before the prover's launch
  (reading the relation and listening).

With --count-bytes the prover connects through a relay that counts the bytes
sent in each direction, including VOLE setup and extension. The relay adds
copying, so time a run without it.
"""

import json
import os
import socket
import subprocess
import sys
import threading
import time


def cpu_ms(rusage):
    return round((rusage.ru_utime + rusage.ru_stime) * 1000)


def relay(listen, target, counts):
    """Accept one connection on `listen` and forward it to `target`."""
    client, _ = listen.accept()
    server = socket.create_connection(target)

    def pump(src, dst, key):
        while True:
            data = src.recv(1 << 16)
            if not data:
                break
            counts[key] += len(data)
            dst.sendall(data)
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass

    threads = [
        threading.Thread(target=pump, args=(client, server, "prover_to_verifier_bytes")),
        threading.Thread(target=pump, args=(server, client, "verifier_to_prover_bytes")),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()


def wait_listening(port, proc, timeout=60):
    """Waits until the verifier listens on `port`. Polls /proc/net/tcp rather
    than connecting, because the verifier takes the first connection as the
    prover's."""
    want = f":{port:04X}"
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise SystemExit("verifier exited before listening")
        for table in ("/proc/net/tcp", "/proc/net/tcp6"):
            try:
                lines = open(table, encoding="ascii").read().splitlines()[1:]
            except FileNotFoundError:
                continue
            for line in lines:
                fields = line.split()
                if fields[1].endswith(want) and fields[3] == "0A":
                    return
        time.sleep(0.01)
    raise SystemExit("verifier did not listen")


def main():
    dietmc, config, irdir, port = sys.argv[1:5]
    port = int(port)
    count = "--count-bytes" in sys.argv[5:]
    common = [dietmc, "--config", config, "--text",
              "--instance", os.path.join(irdir, "public.txt"),
              "--relation", os.path.join(irdir, "relation.txt")]
    v_log = open(os.path.join(irdir, "verifier.log"), "w")
    p_log = open(os.path.join(irdir, "prover.log"), "w")

    v_start = time.monotonic()
    verifier = subprocess.Popen(common + ["-c", f"127.0.0.1:{port}"], stdout=v_log, stderr=v_log)
    wait_listening(port, verifier)

    counts = {"prover_to_verifier_bytes": 0, "verifier_to_prover_bytes": 0}
    prover_port = port
    if count:
        listen = socket.socket()
        listen.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listen.bind(("127.0.0.1", 0))
        listen.listen(1)
        prover_port = listen.getsockname()[1]
        forwarder = threading.Thread(target=relay, args=(listen, ("127.0.0.1", port), counts))
        forwarder.start()

    p_start = time.monotonic()
    prover = subprocess.Popen(
        common + ["--witness", os.path.join(irdir, "private.txt"), "-c", f"127.0.0.1:{prover_port}"],
        stdout=p_log, stderr=p_log)
    ended = {}
    usage = {}
    for _ in range(2):
        pid, status, rusage = os.wait4(-1, 0)
        ended[pid] = time.monotonic()
        usage[pid] = rusage
        if status != 0:
            raise SystemExit(f"dietmc exited with status {status}")
    if count:
        forwarder.join()
    v_log.close()
    if "VERIFIER DONE" not in open(os.path.join(irdir, "verifier.log")).read():
        raise SystemExit("verifier did not accept")

    out = {
        "prover_ms": round((ended[prover.pid] - p_start) * 1000),
        "prover_cpu_ms": cpu_ms(usage[prover.pid]),
        "verifier_cpu_ms": cpu_ms(usage[verifier.pid]),
        "verifier_online_ms": round((ended[verifier.pid] - p_start) * 1000),
        "verifier_setup_ms": round((p_start - v_start) * 1000),
    }
    if count:
        out.update(counts)
    print(json.dumps(out))


if __name__ == "__main__":
    main()
