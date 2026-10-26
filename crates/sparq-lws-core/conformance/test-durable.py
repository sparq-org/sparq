#!/usr/bin/env python3
"""Loopback resource tests and complete driver runs with local suite/process fixtures."""

import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import ssl
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("durable", HERE / "durable-check.py")
durable = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(durable)


def event(value):
    with open(os.environ["FIXTURE_TRACE"], "a") as stream:
        stream.write(json.dumps(value) + "\n")


def fixture_server():
    directory = os.environ.get("SOLID_SERVER_DATA_DIR")
    event({"boot": directory, "backend": os.environ.get("PSS_SPARQ_BACKEND"),
           "seed": os.environ.get("SOLID_SERVER_SEED_CONFORMANCE"),
           "allow_seed": os.environ.get("SOLID_SERVER_ALLOW_SEED_NONMEMORY"),
           "wait": os.environ.get("SOLID_SERVER_WAIT_FOR_LOCK_SECS"), "pid": os.getpid()})
    key = os.environ.get("SOLID_SERVER_LWS_AS_KEY_FILE")
    if key and not Path(key).exists():
        Path(key).write_text('{"fixture":true}')
    def terminate(*_):
        event({"term": os.getpid()})
        sys.exit(0)
    signal.signal(signal.SIGTERM, terminate)
    while True:
        signal.pause()


def fixture_suite(kind, argv):
    event({"suite": kind})
    bad_phase = os.environ.get("FIXTURE_BAD_FLOOR", "")
    if kind == "cth":
        mount = next(value for value in argv if value.endswith(":/reports"))
        output = Path(mount.removesuffix(":/reports"))
        bad = bad_phase == "initial" or (bad_phase and bad_phase in str(output))
        base = json.loads((HERE / "baseline.json").read_text())
        # The driver must independently enforce the real, unchanged floor.
        rows = ["earl:outcome earl:passed .\n"] * (base["expected_total"] - bool(bad))
        if bad:
            rows.append("earl:outcome earl:failed .\n")
        (output / "report.ttl").write_text("".join(rows))
    else:
        output = Path(argv[argv.index("--report-dir") + 1]) / "fresh"
        bad = bad_phase == "initial" or (bad_phase and bad_phase in str(output))
        output.mkdir(parents=True)
        floor = json.loads((HERE / "lws/floor.json").read_text())
        tests = [{"id": name, "outcome": "passed", "level": "MUST"} for name in floor["passing"]]
        for name, why in floor["supported"].items():
            tests.append({"id": name, "outcome": "inapplicable", "level": "MUST",
                          "reason": f"precondition '{why['precondition']}' does not hold: "
                                    f"{why['assertion']} expected {why['expected']}, got PUT"})
        if bad:
            tests[0]["outcome"] = "failed"
        (output / "report.json").write_text(json.dumps({"tests": tests}))
        print("fixture suite completed")


def fixture_command(argv):
    kind, *args = argv
    if kind == "server":
        fixture_server()
    elif kind == "docker":
        if "--skip-teardown" in args:
            fixture_suite("cth", args)
    elif kind == "java":
        fixture_suite("touchstone", args)
    elif kind == "curl":
        if "-w" in args:
            trace = Path(os.environ["FIXTURE_TRACE"])
            boots = [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
            boots = [row for row in boots if "boot" in row]
            try:
                if not boots:
                    sys.exit(7)
                os.kill(boots[-1]["pid"], 0)
            except ProcessLookupError:
                sys.exit(7)
            print("200", end="")
    elif kind == "python":
        if args[0].endswith("/durable-check.py"):
            event({"verify": args[1], "args": args[2:]})
        else:
            os.execv(sys.executable, [sys.executable, *args])


class ResourceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="sparq-check-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.resources = {"/suite.bin": b"suite bytes", "/alice/test/item.bin": b"solid suite bytes"}
        self.requests = []
        self.etag_override = None
        self.omit_etag = False
        self.put_status = 201
        self.allowed = {}
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def answer(self, status, body=b"", etag=True):
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                if etag and not outer.omit_etag:
                    self.send_header("ETag", outer.etag_override or '"' + hashlib.sha256(body).hexdigest() + '"')
                self.end_headers()
                self.wfile.write(body)

            def do_PUT(self):
                outer.requests.append(("PUT", self.path, dict(self.headers)))
                outer.resources[self.path] = self.rfile.read(int(self.headers["Content-Length"]))
                self.answer(outer.put_status)

            def do_POST(self):
                outer.requests.append(("POST", self.path, dict(self.headers)))
                self.rfile.read(int(self.headers["Content-Length"]))
                credentials = base64.b64decode(self.headers["Authorization"].split()[1]).decode()
                identity = credentials.split(":")[0]
                self.answer(200, json.dumps({"access_token": identity + "-token"}).encode())

            def do_GET(self):
                outer.requests.append(("GET", self.path, dict(self.headers)))
                if self.path in outer.allowed and self.headers.get("Authorization", "") not in outer.allowed[self.path]:
                    self.answer(403, etag=False)
                    return
                if self.path == "/.well-known/lws-configuration":
                    self.answer(200, json.dumps({"issuer": outer.base}).encode())
                elif self.path == "/realm/.well-known/openid-configuration":
                    self.answer(200, json.dumps({"token_endpoint": outer.base + "/token"}).encode())
                elif self.path in ("/alice/test/", "/bob/test/"):
                    members = [outer.base + name for name in outer.resources if name.startswith(self.path)]
                    values = [{"@id": name} for name in members]
                    listing = {"@id": outer.base + self.path}
                    if values:
                        listing["http://www.w3.org/ns/ldp#contains"] = values[0] if len(values) == 1 else values
                    self.answer(200, json.dumps(listing).encode())
                elif self.path in outer.resources:
                    self.answer(200, outer.resources[self.path])
                else:
                    self.answer(404, etag=False)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.base = f"http://localhost:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)
        self.key = self.root / "key.json"
        self.key.write_text(json.dumps({"kty": "EC", "crv": "P-256", "kid": "fixture-key",
                                        "d": durable.b64(b"\x01" * 32)}))
        self.env = self.root / "fixture.env"
        self.env.write_text("".join(f"USERS_{user.upper()}_{key}={value}\n"
                            for user in ("alice", "bob") for key, value in
                            (("IDP", self.base + "/realm/"), ("CLIENTID", user), ("CLIENTSECRET", "local-fixture"))))
        evidence = self.root / "runs/one"
        evidence.mkdir(parents=True)
        (evidence / "run.json").write_text(json.dumps({"tests": [{"steps": [
            {"trace": {"method": "PUT", "status": 201, "uri": self.base + path}}
            for path in ("/suite.bin", "/deleted.bin")]}]}))
        self.manifest = self.root / "manifest.json"

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def cli(self, command, protocol="lws", success=True, phase="term"):
        result = subprocess.run([sys.executable, "-B", str(HERE / "durable-check.py"), command,
                                 "--protocol", protocol, "--base", self.base, "--key", str(self.key),
                                 "--owner", "https://fixture.invalid/alice", "--env-file", str(self.env),
                                 "--runs", str(self.root / "runs"), "--manifest", str(self.manifest), "--phase", phase],
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def test_capture_and_read_only_recheck(self):
        self.cli("capture")
        saved = json.loads(self.manifest.read_text())
        self.assertEqual(len(saved["probes"]), 2)
        urls = {row["url"] for row in saved["resources"]}
        self.assertIn(self.base + "/suite.bin", urls)
        self.assertIn(self.base + "/deleted.bin", urls)
        blob = next(row for row in saved["resources"] if row["url"].endswith("blob.bin"))
        self.assertEqual(durable.unb64(blob["body"]), bytes(range(256)) + b"\x00\xff\r\n")
        self.requests.clear()
        self.cli("check")
        self.assertTrue(self.requests)
        self.assertTrue(all(method == "GET" for method, _, _ in self.requests))
        self.assertNotIn("Bearer ", self.manifest.read_text())

    def test_second_capture_retains_old_and_new_writes(self):
        self.cli("capture")
        self.cli("capture", phase="kill")
        probes = json.loads(self.manifest.read_text())["probes"]
        self.assertEqual(len(set(probes)), 4)
        self.assertTrue(any("/term/" in url for url in probes))
        self.assertTrue(any("/kill/" in url for url in probes))
        self.cli("check")

    def test_changed_bytes_fail(self):
        self.cli("capture")
        self.etag_override = '"' + hashlib.sha256(b"suite bytes").hexdigest() + '"'
        self.resources["/suite.bin"] = b"corrupt bytes"
        self.assertIn("bytes changed", self.cli("check", success=False).stderr)

    def test_changed_etag_fails(self):
        self.cli("capture")
        self.etag_override = '"other-validator"'
        self.assertIn("ETag changed", self.cli("check", success=False).stderr)

    def test_missing_resource_fails(self):
        self.cli("capture")
        del self.resources["/suite.bin"]
        self.assertIn("status changed", self.cli("check", success=False).stderr)

    def test_resurrected_delete_fails(self):
        self.cli("capture")
        self.resources["/deleted.bin"] = b"resurrected"
        self.assertIn("status changed", self.cli("check", success=False).stderr)

    def test_empty_manifest_fails(self):
        self.manifest.write_text('{"probes":[],"resources":[]}')
        self.assertIn("empty persistence manifest", self.cli("check", success=False).stderr)

    def test_capture_requires_etags(self):
        self.omit_etag = True
        self.assertIn("lacks ETag", self.cli("capture", success=False).stderr)

    def test_capture_requires_successful_put(self):
        self.put_status = 507
        self.assertIn("returned 507", self.cli("capture", success=False).stderr)

    def test_capture_requires_suite_put_evidence(self):
        (self.root / "runs/one/run.json").write_text('{"tests":[]}')
        self.assertIn("no successful PUT", self.cli("capture", success=False).stderr)

    def test_fixture_jwt_signatures_verify(self):
        signer = durable.Signer(self.root, json.loads(self.key.read_text()))
        token = signer.jwt({"typ": "at+jwt"}, {"sub": "https://fixture.invalid/alice"})
        header, claims, encoded_signature = token.split(".")
        raw = durable.unb64(encoded_signature)
        def integer(data):
            value = data.lstrip(b"\x00") or b"\x00"
            if value[0] & 0x80:
                value = b"\x00" + value
            return b"\x02" + bytes([len(value)]) + value
        parts = integer(raw[:32]) + integer(raw[32:])
        signature = self.root / "signature.der"
        signature.write_bytes(b"\x30" + bytes([len(parts)]) + parts)
        public = self.root / "public.pem"
        public.write_bytes(subprocess.check_output(["openssl", "pkey", "-inform", "DER", "-in",
                                                   str(signer.path), "-pubout"], stderr=subprocess.DEVNULL))
        verified = subprocess.run(["openssl", "dgst", "-sha256", "-verify", str(public), "-signature", str(signature)],
                                  input=(header + "." + claims).encode(), capture_output=True)
        self.assertEqual(verified.returncode, 0, verified.stderr)

    def test_solid_tree_and_fresh_request_bound_auth(self):
        # Exercise the self-signed TLS path used by the real CTH, including the IdP.
        certificate, key = self.root / "cert.pem", self.root / "tls-key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
                        "-nodes", "-days", "1", "-subj", "/CN=localhost", "-keyout", str(key), "-out", str(certificate)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.server.shutdown()
        self.thread.join()
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(certificate, key)
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.base = self.base.replace("http:", "https:")
        self.env.write_text(self.env.read_text().replace("http:", "https:"))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.cli("capture", "solid")
        rows = json.loads(self.manifest.read_text())["resources"]
        self.assertIn(self.base + "/alice/test/item.bin", {row["url"] for row in rows})
        self.assertIn(self.base + "/alice/test/item.bin.acl", {row["url"] for row in rows})
        self.requests.clear()
        self.cli("check", "solid")
        authenticated = [(method, path, headers) for method, path, headers in self.requests if "DPoP" in headers]
        proofs = [headers["DPoP"] for _, _, headers in authenticated]
        self.assertEqual(len(proofs), len(set(proofs)))
        for method, path, headers in authenticated:
            proof = json.loads(durable.unb64(headers["DPoP"].split(".")[1]))
            self.assertEqual(proof["htm"], method)
            self.assertEqual(proof["htu"], self.base + path)
            if method == "GET":
                self.assertEqual(proof["ath"], durable.b64(hashlib.sha256(b"alice-token").digest()))

    def test_solid_reads_with_the_identity_that_has_access(self):
        self.resources["/alice/test/bob.bin"] = b"bob can read"
        self.resources["/alice/test/public.bin"] = b"public can read"
        self.allowed["/alice/test/bob.bin"] = {"DPoP bob-token"}
        self.allowed["/alice/test/public.bin"] = {""}
        self.cli("capture", "solid")
        rows = {row["url"]: row for row in json.loads(self.manifest.read_text())["resources"]}
        self.assertEqual(rows[self.base + "/alice/test/bob.bin"]["identity"], "bob")
        self.assertEqual(rows[self.base + "/alice/test/public.bin"]["identity"], "anonymous")
        self.cli("check", "solid")


class DriverTests(unittest.TestCase):
    def run_driver(self, protocol, bad_floor=False, mode="durable"):
        with tempfile.TemporaryDirectory(prefix="sparq-driver-test-") as directory:
            root = Path(directory)
            harness = root / "conformance"
            shutil.copytree(HERE, harness, ignore=shutil.ignore_patterns("reports", ".cache", "__pycache__"))
            commands = root / "commands"
            commands.mkdir()
            for name, fixture_mode in (("sparq-fixture", "server"), ("docker", "docker"), ("java", "java"),
                                       ("curl", "curl"), ("python3", "python")):
                wrapper = commands / name
                wrapper.write_text(f'#!/usr/bin/env bash\nexec "{sys.executable}" "{Path(__file__).resolve()}" --fixture {fixture_mode} "$@"\n')
                wrapper.chmod(0o755)
            trace = root / "trace.jsonl"
            env = {**os.environ, "PATH": str(commands) + os.pathsep + os.environ["PATH"],
                   "PSS_SPARQ_BACKEND": mode, "SERVER_BIN": str(commands / "sparq-fixture"),
                   "BIN": str(commands / "sparq-fixture"), "TOUCHSTONE": str(root), "SPEC_TESTS": str(root),
                   "OUT": str(root / "out"), "RUNNER_TEMP": str(root), "FIXTURE_TRACE": str(trace),
                   "SOLID_SERVER_SEED_DEMO": "1", "SOLID_SERVER_SPARQ_DIR": str(root / "wrong-index")}
            if bad_floor:
                env["FIXTURE_BAD_FLOOR"] = bad_floor
            script = harness / ("run.sh" if protocol == "solid" else "lws/touchstone.sh")
            try:
                result = subprocess.run(["bash", str(script)], env=env, capture_output=True, text=True, timeout=45)
            finally:
                events = [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
                lingering = []
                for row in events:
                    if "boot" in row:
                        try:
                            os.kill(row["pid"], signal.SIGKILL)
                            lingering.append(row["pid"])
                        except ProcessLookupError:
                            pass
            self.assertFalse(lingering, "driver left a fixture server alive")
            self.assertEqual(result.returncode == 0, not bad_floor, result.stdout + result.stderr)
            self.assertTrue(trace.exists(), result.stdout + result.stderr)
            boots = [row for row in events if "boot" in row]
            if mode != "durable":
                self.assertEqual(len(boots), 1)
                self.assertIsNone(boots[0]["boot"])
                self.assertEqual(boots[0]["backend"], mode)
                self.assertEqual(boots[0]["seed"], "1" if protocol == "solid" else None)
                self.assertEqual(boots[0]["allow_seed"], "1" if protocol == "solid" and mode == "embedded" else None)
                self.assertFalse(any("verify" in row for row in events))
                return
            expected_boots = {"initial": 1, "post-term": 2, "post-kill": 3}.get(bad_floor, 3)
            self.assertEqual(len(boots), expected_boots, events)
            self.assertEqual({row["boot"] for row in boots}, {boots[0]["boot"]})
            self.assertTrue(boots[0]["boot"].startswith(str(root) + "/sparq-conformance."))
            self.assertFalse(Path(boots[0]["boot"]).exists(), "temporary store was not cleaned up")
            self.assertTrue(all(row["backend"] is None and row["wait"] == "10" for row in boots), events)
            self.assertEqual([row["seed"] for row in boots],
                             (["1"] + [None] * (len(boots) - 1)) if protocol == "solid" else [None] * len(boots))
            self.assertEqual([row["allow_seed"] for row in boots], [row["seed"] for row in boots])
            if bad_floor:
                self.assertEqual(sum(row.get("verify") == "check" for row in events), expected_boots - 1)
                return
            # SIGKILL must not invoke graceful shutdown, and each read-only recheck
            # must happen after a new PID boots, before the suite can recreate state.
            self.assertEqual([row["term"] for row in events if "term" in row], [boots[0]["pid"], boots[2]["pid"]])
            order = [next(key for key in ("boot", "suite", "verify", "term") if key in row)
                     + (":" + row["verify"] if "verify" in row else "") for row in events]
            self.assertEqual(order, ["boot", "suite", "verify:capture", "term", "boot", "verify:check", "suite",
                                     "verify:capture", "boot", "verify:check", "suite", "term"])
            verifies = [row for row in events if "verify" in row]
            for row in verifies:
                self.assertEqual(row["args"][row["args"].index("--protocol") + 1], protocol)
            captures = [row["args"] for row in verifies if row["verify"] == "capture"]
            self.assertEqual([args[args.index("--phase") + 1] for args in captures], ["term", "kill"])

    def test_solid_driver(self):
        self.run_driver("solid")

    def test_touchstone_driver(self):
        self.run_driver("lws")

    def test_existing_ephemeral_modes(self):
        for protocol, mode in (("solid", "memory"), ("solid", "embedded"), ("lws", "memory"), ("lws", "embedded")):
            with self.subTest(protocol=protocol, mode=mode):
                self.run_driver(protocol, mode=mode)

    def test_solid_floor_failure_stops_driver(self):
        for phase in ("initial", "post-term", "post-kill"):
            with self.subTest(phase=phase):
                self.run_driver("solid", bad_floor=phase)

    def test_touchstone_floor_failure_stops_driver(self):
        for phase in ("initial", "post-term", "post-kill"):
            with self.subTest(phase=phase):
                self.run_driver("lws", bad_floor=phase)


class WorkflowTests(unittest.TestCase):
    def test_durable_workflow_legs_are_reached(self):
        workflows = HERE.parents[2] / ".github/workflows"
        cth = (workflows / "lws-cth.yml").read_text()
        self.assertIn("        options: [memory, embedded, durable]\n", cth)
        self.assertIn("      PSS_SPARQ_BACKEND: ${{ inputs.sparq_backend }}\n", cth)
        self.assertIn("    if: ${{ inputs.run_harness }}\n", cth)
        cth_step = re.search(r"      - name: run the Solid CTH\n(.*?)(?=      - )", cth, re.S)
        self.assertIsNotNone(cth_step, "CTH workflow invocation is absent")
        self.assertEqual(cth_step.group(1).rstrip(), "        run: ./crates/sparq-lws-core/conformance/run.sh")
        lws = (workflows / "lws-conformance.yml").read_text()
        job = lws.split("  touchstone:\n", 1)[1].split("  lws-net:\n", 1)[0]
        preamble = "\n".join(line for line in job.split("    steps:\n", 1)[0].splitlines()
                             if not line.lstrip().startswith("#"))
        self.assertEqual(preamble, """    if: github.event_name != 'pull_request' || github.event.pull_request.draft != true
    name: ${{ matrix.name }}
    runs-on: ubuntu-latest
    timeout-minutes: 45
    strategy:
      fail-fast: false
      matrix:
        include:
          - backend: memory
            name: lws touchstone
          - backend: durable
            name: lws touchstone durable
    env:
      PSS_SPARQ_BACKEND: ${{ matrix.backend }}
      OUT: ${{ github.workspace }}/target/lws-touchstone-${{ matrix.backend }}""")
        run_step = re.search(r"      - name: run touchstone\n(.*?)(?=      - )", job, re.S)
        self.assertIsNotNone(run_step, "Touchstone workflow invocation is absent")
        self.assertEqual(run_step.group(1), """        env:
          TOUCHSTONE: ${{ github.workspace }}/touchstone
        run: |
          if [ "$PSS_SPARQ_BACKEND" = durable ]; then
            crates/sparq-lws-core/conformance/lws/touchstone.sh all
          else
            crates/sparq-lws-core/conformance/lws/touchstone.sh all || true
          fi
""")


if __name__ == "__main__":
    if sys.argv[1:2] == ["--fixture"]:
        fixture_command(sys.argv[2:])
    else:
        unittest.main()
