# agent-telemetry — per-agent / per-wave token + cost telemetry

Phase-1 measurement tool for the agent-efficiency program
(`research/agent-efficiency-tooling.md`, §8/§10; bead `sq-dhss`, epic `sq-lhwo`).

The program is **telemetry-first**: *measure before optimizing*, so every later
agent-efficiency percentage claim is grounded on **our** repo's numbers rather than
borrowed from someone else's benchmark. This script is that baseline instrument.

## What it does

Parses a Claude Code transcript JSONL and aggregates token usage:

- **Per agent** — bucketed by `agentId` / `attributionAgent` (the orchestrator's own
  turns are bucketed under `<orchestrator>`).
- **Rolled up per wave / session.**

For each bucket it reports input / output tokens, **cache-read vs cache-creation**
tokens (and the 5-min vs 1-hour write TTL split), the **cache-hit ratio**
(`cache_read / (cache_read + cache_creation)` — the highest-signal efficiency lever
per the design doc's cost model), assistant-turn and tool-call counts, and a
wall-clock duration derived from record timestamps.

Output is a structured JSON report (`--json PATH`) plus a human-readable summary
table.

## Usage

```sh
# tokens + cache-hit ratio only (always honest, no prices needed)
python3 scripts/agent-telemetry/agent_telemetry.py TRANSCRIPT.jsonl

# also write a machine-readable report
python3 scripts/agent-telemetry/agent_telemetry.py TRANSCRIPT.jsonl --json report.json

# add an optional $ estimate (prices are $ per 1M tokens; you supply them)
python3 scripts/agent-telemetry/agent_telemetry.py TRANSCRIPT.jsonl \
    --price-input <N> --price-output <N>
# or from a file: {"input": <N>, "output": <N>[, "cache_read": <N>, "cache_write": <N>]}
python3 scripts/agent-telemetry/agent_telemetry.py TRANSCRIPT.jsonl --prices prices.json
```

Transcript JSONL files live under Claude Code's session directory (e.g.
`~/.claude/projects/<project>/<session>.jsonl` and the per-task `*.output` files).
Stdlib-only; no third-party dependencies and nothing added to `sparq-core`/`-engine`.

### Sibling: the per-PR/task metrics-collection harness

This script is the **token-accounting engine** that the per-PR/task metrics harness
[`metrics_row.py`](./metrics_row.py) **reuses** (it never re-derives token counts).
`metrics_row.py` joins this telemetry with `gh` + `roborev` + `bd` into one structured
row per PR — first-shot-success, ref-event-derived rework, review pushback, CI
first-pass, and the cache-discounted effective-input tokens — and the captured
baseline + the shared A/B protocol live under [`../../metrics/`](../../metrics/README.md)
(bead `sq-lhwo.1`). Start there to run an A/B; the row schema is defined in that README.

### Cost is opt-in and never hard-coded

The tool does **not** bake in vendor prices — they drift, and a stale price inside a
measurement tool is itself a lie. With no price flags it reports tokens and the
cache-hit ratio only. Supply prices explicitly to get a `$` estimate; if you give
only `--price-input`, cache read / write prices default to the documented
multipliers (cache read = a fraction of input; cache write = a multiple of input)
unless you override them.

## Honesty: numbers are NON-CANONICAL

Any token / cost / duration number this tool prints is a **work-box / session-local
measurement** and is **non-canonical** (see MEMORY: `project-ec2-execution-env`).
The tool reports numbers **at runtime only**. Do **not** copy a measured number into
committed markdown or docs as if it were a canonical benchmark. The committed
artifacts are the **code** and the **synthetic-fixture test** — never a real session
transcript (those may carry repo-internal detail) and never a measured number in
docs.

## Tests

Hermetic stdlib `unittest` over a checked-in synthetic fixture transcript:

```sh
python3 scripts/agent-telemetry/tests/test_agent_telemetry.py
```

The fixture (`tests/fixture_transcript.jsonl`) is a handful of synthetic JSONL lines
(orchestrator + two sub-agents, a tool-use block, plus malformed / bare-string /
usage-less lines to cover robustness). It contains **no** real session data.
