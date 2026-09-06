#!/usr/bin/env python3
"""[GPT-6] Verify real loopback requests for both persisted policy variants."""

import argparse
import json
import pathlib
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("--workload", type=pathlib.Path, required=True)
    arguments = parser.parse_args()
    root = pathlib.Path(tempfile.mkdtemp(prefix="sparq-pop-http-"))
    binary = str(arguments.binary.resolve())

    def run(command, output):
        with (root / output).open("w") as stream:
            subprocess.run([binary, *command], stdout=stream, stderr=subprocess.STDOUT, check=True)

    run(["auth", "--auth-dir", str(root / "auth")], "auth.jsonl")
    workload = json.loads(arguments.workload.read_text())
    for journey in workload["journeys"]:
        journey["actions_per_active_person_day"] = 0
    workload["background"].update(
        read_requests_per_hosted_person_day=0,
        content_write_requests_per_active_person_day=0,
        policy_changes_per_hosted_person_day=1,
    )
    (root / "policy-only.json").write_text(json.dumps(workload))
    with socket.socket() as port_probe:
        port_probe.bind(("127.0.0.1", 0))
        port = port_probe.getsockname()[1]
    address = f"127.0.0.1:{port}"
    for model in ("wac", "acp"):
        corpus = root / model
        run(["pack", "--corpus", str(corpus), "--profile", "smoke", "--pods", "8", "--model", model], f"{model}-pack.jsonl")
        run(["verify", "--corpus", str(corpus), "--verify-pods", "8", "--workload-file", str(arguments.workload)], f"{model}-verify.jsonl")
        with (root / f"{model}-server.jsonl").open("w") as stream:
            server = subprocess.Popen(
                [binary, "serve", "--corpus", str(corpus), "--auth-dir", str(root / "auth"),
                 "--bind", address, "--cache-pods", "1"], stdout=stream, stderr=subprocess.STDOUT,
            )
            try:
                for _ in range(50):
                    if server.poll() is not None:
                        raise RuntimeError(f"server exited; inspect {root}")
                    try:
                        urllib.request.urlopen(f"http://{address}/", timeout=0.1)
                    except urllib.error.HTTPError:
                        break
                    except urllib.error.URLError:
                        time.sleep(0.1)
                else:
                    raise RuntimeError("server did not become ready")
                for index, (state, delta) in enumerate((("grant", "0"), ("revoke", "-1"), ("probe-revoked", None), ("probe-revoked", None), ("grant", "1"))):
                    if index == 3:
                        previous_args=server.args
                        server.terminate();server.wait(timeout=5)
                        server=subprocess.Popen(previous_args,stdout=stream,stderr=subprocess.STDOUT)
                        for _ in range(50):
                            try: urllib.request.urlopen(f"http://{address}/",timeout=.1)
                            except urllib.error.HTTPError: break
                            except urllib.error.URLError: time.sleep(.1)
                    command=["churn", "--corpus", str(corpus), "--auth-dir", str(root / "auth"), "--connect", f"http://{address}", "--state", state, "--mutation-id", f"churn-{index}"]
                    if delta is not None: command += ["--expected-delta", delta]
                    elif index == 2: command += ["--evict-pod", "1"]
                    run(command, f"{model}-churn-{index}.jsonl")
                # The eviction/restart lane above deliberately uses one entry. Load
                # correctness uses all eight tiny fixtures; this is not a capacity run.
                previous_args=list(server.args)
                previous_args[previous_args.index("--cache-pods")+1]="8"
                server.terminate();server.wait(timeout=5)
                server=subprocess.Popen(previous_args,stdout=stream,stderr=subprocess.STDOUT)
                for _ in range(50):
                    try: urllib.request.urlopen(f"http://{address}/",timeout=.1)
                    except urllib.error.HTTPError: break
                    except urllib.error.URLError: time.sleep(.1)
                common = ["load", "--corpus", str(corpus), "--auth-dir", str(root / "auth"),
                          "--connect", f"http://{address}", "--query-set", "population",
                          "--arrival", "poisson", "--rate", "20"]
                run(common + ["--pods", "8", "--requests", "32", "--mix", "population",
                              "--workload-file", str(arguments.workload), "--out", str(root / f"{model}-requests.jsonl")], f"{model}-load.txt")
                run(common + ["--pods", "1", "--requests", "4", "--mix", "population",
                              "--workload-file", str(root / "policy-only.json"), "--out", str(root / f"{model}-policy.jsonl")], f"{model}-policy-load.txt")
                run(common + ["--requests", "512", "--mix", "journeys", "--seed", "2026090601",
                              "--mutation-epoch", "1", "--workload-file", str(arguments.workload),
                              "--out", str(root / f"{model}-journeys.jsonl")], f"{model}-journeys-load.txt")
                for label in ("requests", "policy", "journeys"):
                    rows = [json.loads(line) for line in (root / f"{model}-{label}.jsonl").read_text().splitlines()]
                    rows = [row for row in rows if row["record_type"] == "request"]
                    bad=[row for row in rows if row.get("status")!=200]
                    assert not bad, {"failed":len(bad),"sample":bad[:3],"artifacts":str(root)}
                    if label == "journeys":
                        mutations = [row for row in rows if row.get("planned_records")]
                        assert mutations, "smoke seed must exercise real mutations"
                        assert all(row.get("mutation_receipt_present") for row in mutations), mutations
                    if label == "policy":
                        deltas = [row["policy_triple_delta"] for row in sorted(rows, key=lambda row: row["sequence"])]
                        assert deltas == [-1, 1, -1, 1], rows
                    print(model, label, len(rows), "all HTTP 200", flush=True)
            finally:
                server.terminate()
                server.wait(timeout=5)
        run(["audit", "--corpus", str(corpus)], f"{model}-audit.jsonl")
    print("ARTIFACTS", root)


if __name__ == "__main__":
    main()
