#!/usr/bin/env python3
"""Execute the paper corpus repeatedly, varying the SDK startup entropy.

The pinned x86-64 executor's private SysRandom handler is observed directly.
Exactly its startup request is allowed; a later request stops the executor.
No proof is produced. Missing symbols, debugger failures and incomplete corpora
are errors, never reasons to skip the check.
"""

import argparse
import inspect
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
MANIFEST = ROOT / "zk/sparql-evaluator/Cargo.toml"
SAMPLES = (0, 1, 127, 254, 255)
KEY = ("set", "id", "suite", "mode", "n", "statements_per_credential")


def snapshot(record):
    """Keep identities, outcomes and every executor/phase count, without timers."""
    result = {name: record[name] for name in KEY}
    for name in ("witness_sha256", "artifact_sha256"):
        result[name] = record[name]
    for name in ("admitted", "query_profile", "rejection", "result_bytes", "guest_aborted"):
        if name in record:
            result[name] = record[name]
    if "guest_error" in record:
        raise ValueError(record["guest_error"])
    if record["admitted"]:
        for name in ("execution", "phase_execution"):
            value = dict(record[name])
            value.pop("execute_ns", None)
            if "error" in value:
                raise ValueError(value["error"])
            for field in ("user_cycles", "padded_cycles", "segments", "witness_sha256", "artifact_sha256", "journal_sha256"):
                if field not in value:
                    raise ValueError(f"missing {name}.{field}")
            result[name] = value
        counts = record["phase_cycles"]
        if not counts or any(type(value) is not int for value in counts.values()):
            raise ValueError("missing or invalid phase counts")
        result["phase_cycles"] = counts
        if result["execution"]["journal_sha256"] != result["phase_execution"]["journal_sha256"]:
            raise ValueError("production and phase journals differ")
        if result["execution"]["journal_sha256"] != record.get("native_journal_sha256"):
            raise ValueError("guest and native journals differ")
        if any(result[name]["witness_sha256"] != result["witness_sha256"]
               for name in ("execution", "phase_execution")):
            raise ValueError("execution witness identities differ")
        if result["execution"]["artifact_sha256"] != result["artifact_sha256"]:
            raise ValueError("execution artifact identity differs")
    elif record.get("guest_aborted") is not True:
        raise ValueError("native rejection has no matching guest abort")
    return result


def census(records, campaign):
    keys = [tuple(record[name] for name in KEY) for record in records]
    if len(keys) != len(set(keys)):
        raise ValueError("duplicate corpus record")
    fixtures = ROOT / "zk/sparql-evaluator/fixtures/paper"
    if campaign == "main":
        ids = {path.stem for path in fixtures.glob("*.rq")}
        expected = {(campaign, name, suite, mode, n, 17) for name in ids
                    for suite in ("eddsa-rdfc-2022", "eddsa-sha256-merkle-2026")
                    for mode in ("hidden", "revealed") for n in (1, 4)}
        if len(expected) != 40:
            raise ValueError("main corpus membership changed; review the invariant")
    elif campaign == "sweep":
        ids = {case["id"] for case in json.loads((fixtures / "sweep.json").read_text())["cases"]}
        if len(ids) != 52:
            raise ValueError("sweep membership changed; review the invariant")
        expected = {(campaign, name, "eddsa-rdfc-2022", "revealed", 1, 32) for name in ids}
    else:
        ids = {case["id"] for case in json.loads((fixtures / "determinism.json").read_text())["cases"]}
        if campaign != "determinism" or len(ids) != 6:
            raise ValueError("operator guard membership changed")
        expected = {(campaign, name, "eddsa-rdfc-2022", "revealed", 1, 32) for name in ids}
    if set(keys) != expected:
        raise ValueError("incomplete or unexpected corpus grid")


def compare(expected, actual):
    if expected != actual:
        raise ValueError("same image and witness produced different outcomes, counts or journal hashes")


def canonical_inputs():
    root = ROOT / "crates/sparq-canon/tests/rdf-canon-testdata"
    entries = re.findall(r"(?ms)^:(test\w+) a rdfc:(RDFC10\w+);(.*?)(?=^:|\Z)",
                         (root / "manifest.ttl").read_text())
    if len(entries) != 86:
        raise ValueError("canonicalisation suite membership changed")
    inputs = []
    for name, kind, body in entries:
        action = re.search(r"mf:action <([^>]+)>", body).group(1)
        item = {"id": name, "input": (root / action).read_text(),
                "sha384": 'rdfc:hashAlgorithm "SHA384"' in body}
        if kind == "RDFC10EvalTest":
            result = re.search(r"mf:result <([^>]+)>", body).group(1)
            item["expected"] = (root / result).read_text()
        elif kind == "RDFC10NegativeEvalTest":
            item["negative"] = True
        inputs.append(item)
    for fixture in ("default.nt", "conformance/core.nt"):
        path = ROOT / "zk/sparql-evaluator/fixtures" / fixture
        inputs.append({"id": fixture, "input": path.read_text(), "sha384": False})
    return inputs


def canonical_differential(output, binary, runtime):
    inputs = output / "canonical-inputs.json"
    inputs.write_text(json.dumps(canonical_inputs()) + "\n")
    counters = output / "canonical-randomness.jsonl"
    with tempfile.TemporaryDirectory(prefix="sparq-canon-differential-") as tmp:
        driver = wrapper(Path(tmp), runtime, 0, counters)
        env = dict(os.environ, RISC0_SERVER_PATH=str(driver))
        with (output / "canonical-differential.jsonl").open("w") as stdout:
            subprocess.run([str(binary), str(inputs)], env=env, stdout=stdout, check=True)
    observations = [json.loads(line) for line in counters.read_text().splitlines()]
    validate_observations(observations, len(canonical_inputs()), (0,))


def startup_request(calls, words, guest_pc, init_range):
    """Permit only the SDK initialization request in the decoded guest image."""
    return (calls == 1 and words == 4 and type(guest_pc) is int
            and isinstance(init_range, (list, tuple)) and len(init_range) == 2
            and init_range[0] <= guest_pc < init_range[1])


# Rust's Result return value uses a hidden result pointer on this pinned
# x86-64 binary. The final slice argument is at rsp+8 (pointer), rsp+16 (words).
# Refuse any other ABI/handler shape instead of silently observing nothing.
DEBUGGER = r'''
import gdb, hashlib, json, struct, subprocess, tempfile
calls = 0
error = None
exit_code = None
entropy_digest = None
guest_pc = None
init_range = None
artifact_digest = None

def exited(event):
    global exit_code
    exit_code = getattr(event, "exit_code", None)
gdb.events.exited.connect(exited)

class Completed(gdb.FinishBreakpoint):
    def __init__(self, pointer, size):
        super().__init__(gdb.newest_frame(), internal=True)
        self.pointer, self.size = pointer, size
    def stop(self):
        global entropy_digest
        payload = hashlib.sha256(str(SEED).encode()).digest()[:self.size]
        gdb.selected_inferior().write_memory(self.pointer, payload)
        observed = bytes(gdb.selected_inferior().read_memory(self.pointer, self.size))
        entropy_digest = hashlib.sha256(observed).hexdigest()
        return False

class Random(gdb.Breakpoint):
    def stop(self):
        global calls, error, guest_pc
        calls += 1
        words = int(gdb.parse_and_eval('*(unsigned long*)($rsp+16)'))
        # SyscallContext is passed in r8/r9. Verify the pinned get_pc accessor
        # before following its read-only pointer/field layout; never call into
        # the inferior from a breakpoint callback.
        context = int(gdb.parse_and_eval('$r8'))
        vtable = int(gdb.parse_and_eval('$r9'))
        accessor = int(gdb.parse_and_eval(f'*(unsigned long*)({vtable}+24)'))
        if bytes(gdb.selected_inferior().read_memory(accessor, 10)).hex() != '488b078b8060030000c3':
            error = "unsupported guest PC accessor"
            return True
        executor = int(gdb.parse_and_eval(f'*(unsigned long*){context}'))
        guest_pc = int(gdb.parse_and_eval(f'*(unsigned int*)({executor}+0x360)'))
        if not startup_request(calls, words, guest_pc, init_range):
            error = "unexpected guest randomness request"
            return True
        pointer = int(gdb.parse_and_eval('*(unsigned long*)($rsp+8)'))
        Completed(pointer, words * 4)
        return False

class Binary(gdb.Breakpoint):
    def stop(self):
        global error, init_range, artifact_digest
        try:
            pointer = int(gdb.parse_and_eval('$rsi'))
            size = int(gdb.parse_and_eval('$rdx'))
            if not 16 <= size <= 64 * 1024 * 1024:
                raise ValueError("unsupported program binary size")
            blob = bytes(gdb.selected_inferior().read_memory(pointer, size))
            if blob[:8] != b'R0BF\x01\x00\x00\x00':
                raise ValueError("unsupported program binary header")
            offset = 12 + struct.unpack_from('<I', blob, 8)[0]
            length = struct.unpack_from('<I', blob, offset)[0]
            user = blob[offset + 4:offset + 4 + length]
            if len(user) != length or user[:4] != b'\x7fELF':
                raise ValueError("missing user ELF")
            with tempfile.NamedTemporaryFile() as elf:
                elf.write(user)
                elf.flush()
                table = subprocess.check_output(['nm', '-S', '-C', elf.name], text=True)
            matches = [line.split() for line in table.splitlines()
                       if line.endswith(' risc0_zkvm::guest::env::init')]
            if len(matches) != 1 or len(matches[0]) != 4:
                raise ValueError("missing unique guest initialization symbol")
            start, length = (int(value, 16) for value in matches[0][:2])
            if not length:
                raise ValueError("empty guest initialization symbol")
            init_range = (start, start + length)
            artifact_digest = hashlib.sha256(blob).hexdigest()
            return False
        except Exception as failure:
            error = str(failure)
            return True

symbols = subprocess.check_output(["nm", "--defined-only", gdb.current_progspace().filename], text=True)
names = [line.split()[-1] for line in symbols.splitlines()
         if len(line.split()) == 3 and line.split()[1] in ("t", "T")
         and "SysRandom" in line and "syscall" in line and "CALLSITE" not in line]
if len(names) != 1:
    raise RuntimeError("expected the pinned executor's unique SysRandom handler")
Random("*" + names[0], internal=True)
decoders = [line.split()[-1] for line in symbols.splitlines()
            if len(line.split()) == 3 and line.split()[1] in ('t', 'T')
            and 'ProgramBinary' in line and line.split()[-1].endswith('6decode')]
if len(decoders) != 1:
    raise RuntimeError("expected the pinned program binary decoder")
Binary("*" + decoders[0], internal=True)

def finish():
    with open(COUNTERS, "a") as output:
        output.write(json.dumps({"startup_pattern": SEED, "randomness_calls": calls,
                                 "startup_entropy_sha256": entropy_digest,
                                 "guest_pc": guest_pc, "init_range": init_range,
                                 "artifact_sha256": artifact_digest,
                                 "exit_code": exit_code, "error": error}) + "\n")
    if error is not None or calls != 1 or exit_code != 0:
        gdb.execute("quit 1")
'''


def validate_observations(observations, per_sample, samples):
    if len(observations) != per_sample * len(samples):
        raise ValueError("missing or extra executor syscall observations")
    digests = set()
    for seed in samples:
        observed = [row for row in observations if row["startup_pattern"] == seed]
        if len(observed) != per_sample or any(
            row["randomness_calls"] != 1 or row["error"] or row["exit_code"] != 0
            or not row.get("startup_entropy_sha256") for row in observed
        ):
            raise ValueError("guest randomness invariant failed")
        if any(not startup_request(1, 4, row.get("guest_pc"), row.get("init_range"))
               or not row.get("artifact_sha256") for row in observed):
            raise ValueError("randomness request is not in guest initialization")
        actual = {row["startup_entropy_sha256"] for row in observed}
        if len(actual) != 1:
            raise ValueError("startup reply changed within a sample")
        digests.update(actual)
    if len(digests) != len(samples):
        raise ValueError("startup replies did not vary across samples")


def wrapper(directory, runtime, seed, counters):
    observer = directory / "observer.py"
    observer.write_text(f"SEED = {seed!r}\nCOUNTERS = {str(counters)!r}\n"
                        + inspect.getsource(startup_request) + "\n" + DEBUGGER)
    commands = directory / "observer.gdb"
    commands.write_text(
        "set pagination off\nset confirm off\nset debuginfod enabled off\n"
        "set print thread-events off\nset disable-randomization off\n"
        f"python\nexec(open({str(observer)!r}).read())\nend\nrun\npython\nfinish()\nend\n"
    )
    driver = directory / "r0vm-observed"
    driver.write_text(
        "#!/usr/bin/env python3\nimport os, subprocess, sys\n"
        f"runtime = {str(runtime)!r}\n"
        "if sys.argv[1:] == ['--version']:\n"
        "    raise SystemExit(subprocess.call([runtime, '--version']))\n"
        f"with open({str(counters.parent / 'debugger.log')!r}, 'a') as log:\n"
        f"    result = subprocess.call(['gdb', '--nx', '--quiet', '--batch', '-x', {str(commands)!r}, "
        "'--args', runtime, *sys.argv[1:]], stdout=log, stderr=log, "
        "env=dict(os.environ, DEBUGINFOD_URLS=''))\n"
        "raise SystemExit(result)\n"
    )
    driver.chmod(0o755)
    return driver


def exact_differential(output, binary, runtime):
    counters = output / "exact-randomness.jsonl"
    previous = None
    with tempfile.TemporaryDirectory(prefix="sparq-exact-determinism-") as tmp:
        for seed in SAMPLES:
            driver = wrapper(Path(tmp), runtime, seed, counters)
            raw = output / f"exact-{seed}.jsonl"
            with raw.open("w") as stdout:
                subprocess.run([str(binary)], stdout=stdout, check=True,
                               env=dict(os.environ, RISC0_SERVER_PATH=str(driver)))
            with raw.open() as source:
                records = [json.loads(line) for line in source]
            if len(records) != 2 or {row["version"] for row in records} != {1, 2}:
                raise ValueError("incomplete production exact-image entry coverage")
            if previous is not None:
                compare(previous, records)
            previous = records
    with counters.open() as source:
        validate_observations([json.loads(line) for line in source], 2, SAMPLES)


def run(output, binary, runtime):
    output.mkdir(parents=True, exist_ok=False)
    exact_differential(output, binary.with_name("exact_determinism"), runtime)
    previous = {}
    counters = output / "randomness.jsonl"
    with tempfile.TemporaryDirectory(prefix="sparq-guest-determinism-") as tmp:
        directory = Path(tmp)
        for seed in SAMPLES:
            driver = wrapper(directory, runtime, seed, counters)
            env = dict(os.environ, RISC0_SERVER_PATH=str(driver))
            for campaign in ("main", "sweep", "determinism"):
                command = [str(binary), "--set", campaign, "--execute", "--phases"]
                if campaign != "main":
                    command += ["--suite", "eddsa", "--mode", "revealed", "--n", "1", "--size", "32"]
                raw = output / f"{campaign}-{seed}.jsonl"
                with raw.open("w") as stdout, raw.with_suffix(".log").open("w") as stderr:
                    subprocess.run(command, env=env, stdout=stdout, stderr=stderr, check=True)
                records = [json.loads(line) for line in raw.read_text().splitlines()]
                census(records, campaign)
                current = sorted((snapshot(record) for record in records), key=lambda r: tuple(r[k] for k in KEY))
                if campaign in previous:
                    compare(previous[campaign], current)
                else:
                    previous[campaign] = current
    observations = [json.loads(line) for line in counters.read_text().splitlines()]
    per_sample = sum(2 if row["admitted"] else 1 for rows in previous.values() for row in rows)
    validate_observations(observations, per_sample, SAMPLES)
    (output / "summary.json").write_text(json.dumps({
        "samples": len(SAMPLES), "records": {key: len(rows) for key, rows in previous.items()},
        "randomness_calls_per_execution": 1, "application_randomness_calls": 0,
        "startup_entropy_varied": True, "result": "execution_repeats_passed",
        "production_exact_versions": [1, 2],
        "rejected_cases": "identical aborts; the SDK returns no session counts or journal",
    }, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--binary", type=Path, help="already built paper_sweep executable")
    args = parser.parse_args()
    runtime = Path(os.environ["RISC0_SERVER_PATH"]).resolve()
    version = subprocess.check_output([str(runtime), "--version"], text=True).strip()
    if version != "risc0-r0vm 3.0.6":
        raise ValueError("the syscall observer requires the pinned runtime")
    binary = args.binary
    if binary is None:
        subprocess.run(["cargo", "build", "--locked", "--manifest-path", str(MANIFEST),
                        "-p", "sparq-proved-evaluator", "--features", "phase-cycles",
                        "--example", "paper_sweep", "--example", "canon_differential",
                        "--example", "exact_determinism"], check=True)
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", str(MANIFEST)]))
        binary = Path(metadata["target_directory"]) / "debug/examples/paper_sweep"
    # Preserve the caller's writable mount path rather than resolving it to a
    # potentially read-only backing-store alias.
    output = args.output.absolute()
    run(output, binary.resolve(), runtime)
    canonical_differential(output, binary.resolve().with_name("canon_differential"), runtime)
    summary_path = output / "summary.json"
    summary = json.loads(summary_path.read_text())
    summary.update(result="passed", canonical_differential="passed")
    summary_path.write_text(json.dumps(summary, indent=2) + "\n")
    print("guest execution repeatability passed")


if __name__ == "__main__":
    main()
