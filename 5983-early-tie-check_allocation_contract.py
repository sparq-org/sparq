"""[GPT-6 Astra] Assert saved executed-binary allocation work, not elapsed time."""
from pathlib import Path
import json
import sys

case, variant = sys.argv[1:]
assert case in ("ties", "ties-overlay")
assert variant in ("new", "early-disabled")
data = json.loads((Path(__file__).parent / "allocation-control.json").read_text())
check = next(c for c in data["checks"] if c["case"] == case)
candidate = check["new" if variant == "new" else "early_disabled"]
late = check["early_disabled"]
assert late["requested_bytes"] - candidate["requested_bytes"] >= check["minimum_avoided_bytes"], f"{case}: oversized leading group must decline before retaining six subject vectors"
assert candidate["allocs"] < late["allocs"], f"{case}: preparation allocations must be avoided"
print(f"PASS {case}: avoided preparation allocation contract ({variant})")
