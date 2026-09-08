"""[GPT-6 Astra] Fixed-work allocation control; no wall-clock assertions."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import time

out = Path(__file__).parent
matrix = json.loads((out / "measurements.json").read_text())
binary = out / "bins/early-disabled-count"
metrics = ["allocs", "reallocs", "requested_bytes", "peak_live_delta"]
records = []
checks = []

def measured(record):
    samples = [r for r in record["rows"] if r["kind"] == "sample"]
    assert len(samples) == 3
    assert len({tuple(s[k] for k in metrics) for s in samples}) == 1
    return {k: samples[0][k] for k in metrics}

for case in ["base", "ties", "ties-overlay"]:
    expected = "indexed" if case == "base" else "fallback"
    for mode in ["verify", "measure"]:
        command = [str(binary), mode, case, "50000", "1", "0" if mode == "verify" else "2", "1" if mode == "verify" else "3", expected]
        started = time.time()
        result = subprocess.run(command, capture_output=True, text=True, timeout=120, env=dict(os.environ, RAYON_NUM_THREADS="1"))
        raw = out / "raw" / f"allocation-control-{case}-{mode}.txt"
        raw.write_text(result.stdout + result.stderr)
        rows = []
        for line in result.stdout.splitlines():
            if not line.startswith("kind="):
                continue
            row = {}
            for field in line.split("\t"):
                key, value = field.split("=", 1)
                try:
                    value = int(value)
                except ValueError:
                    pass
                row[key] = value
            rows.append(row)
        record = dict(case=case, mode=mode, command=command, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), exit_code=result.returncode, started_unix=started, wall_seconds=time.time()-started, rows=rows, raw=str(raw))
        records.append(record)
        (out / "allocation-control.json").write_text(json.dumps(dict(records=records, checks=checks), indent=2) + "\n")
        assert result.returncode == 0, "Stop on a failed harness/control; no retry"
    old_placement = measured(record)
    new = measured(next(r for r in matrix if r["case"] == case and r["k"] == 1 and r["mode"] == "count" and r["variant"] == "new"))
    frozen_old = measured(next(r for r in matrix if r["case"] == case and r["k"] == 1 and r["mode"] == "count" and r["variant"] == "old"))
    if case == "base":
        assert new == old_placement == frozen_old
        checks.append(dict(case=case, positive_control="identical query allocation metrics", metrics=new))
        continue
    seed = next(r["seed_rows"] for r in record["rows"] if r["kind"] == "fixture")
    minimum_avoided_bytes = 6 * seed * 4  # Six retained subject-id vectors; Id is u32.

    def avoids_probe_preparation(candidate):
        return old_placement["requested_bytes"] - candidate["requested_bytes"] >= minimum_avoided_bytes and candidate["allocs"] < old_placement["allocs"]

    assert avoids_probe_preparation(new)
    assert not avoids_probe_preparation(old_placement), "Control must kill the allocation-saving assertion"
    assert frozen_old == old_placement
    checks.append(dict(case=case, seed_rows=seed, minimum_avoided_bytes=minimum_avoided_bytes, avoided_requested_bytes=old_placement["requested_bytes"]-new["requested_bytes"], avoided_allocations=old_placement["allocs"]-new["allocs"], new=new, early_disabled=old_placement, frozen_old_equal=True, guard_disabled_mutation="killed by avoided-work assertion", peak_live_delta_change=new["peak_live_delta"]-old_placement["peak_live_delta"]))

(out / "allocation-control.json").write_text(json.dumps(dict(records=records, checks=checks, wall_clock_assertions=False), indent=2) + "\n")
print(json.dumps(checks, indent=2))
