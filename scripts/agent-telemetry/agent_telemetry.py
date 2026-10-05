#!/usr/bin/env python3
# Agent-wave token/cost telemetry harness (bead sq-dhss, epic sq-lhwo).
#
# WHY THIS EXISTS
# --------------------------------------------------------------------------------
# The agent-efficiency program (research/agent-efficiency-tooling.md, §8/§10 Phase 1)
# is TELEMETRY-FIRST: "measure before optimizing — so every later %-claim is on OUR
# repo, not borrowed." This harness is that Phase-1 measurement tool. It parses a
# Claude Code transcript JSONL and aggregates token usage PER agent (by agentId /
# attributionAgent) and rolled up PER session/wave, surfacing the cache-hit ratio
# (cache_read vs cache_creation) — the single highest-signal efficiency lever per
# the design doc's cost model (§1: a cache read costs ~0.1x base input, a cache
# write ~1.25-2x; a fan-out of cold sub-agent/worktree caches is the root cost).
#
# HONESTY / NON-CANONICAL (load-bearing)
# --------------------------------------------------------------------------------
# Any token / cost / duration number this tool PRINTS is a work-box / session-local
# measurement and is NON-CANONICAL (per MEMORY.md: project-ec2-execution-env). It is
# reported at runtime ONLY. NOTHING measured is baked into committed markdown. The
# committed artifacts are this CODE and the synthetic-fixture test — never a real
# session transcript (those may carry repo-internal detail) and never a measured
# number in docs (scripts/check-no-perf-numbers.py --enforce guards the README).
#
# COST MODEL
# --------------------------------------------------------------------------------
# Cost is OPTIONAL and only computed when the caller passes an explicit price table
# (--prices PATH or the built-in --price-* flags). We DO NOT hard-code vendor prices:
# they drift, and a stale price baked into a measurement tool is itself a lie. With
# no prices the tool reports tokens + cache-hit ratio only (always honest). With a
# price table it additionally reports a $ estimate using the documented multipliers
# (cache read = read_multiplier x input; cache write = write_multiplier x input).
#
# DATA MODEL (verified against real transcripts on this checkout)
# --------------------------------------------------------------------------------
# Each transcript line is a JSON object. The records we care about:
#   type == "assistant"  -> message.usage carries the per-turn token counts:
#       input_tokens, output_tokens, cache_read_input_tokens,
#       cache_creation_input_tokens, and cache_creation.{ephemeral_5m,1h}_input_tokens
#   Attribution lives at the TOP level of the record:
#       agentId           -> stable id of the sub-agent that produced the turn
#       attributionAgent  -> the agent's slug/type (e.g. "sparq-rust-feature", "fork")
#       isSidechain       -> True for sub-agent turns, False/absent for the orchestrator
#       sessionId, timestamp, message.model
# The orchestrator's own turns have no agentId; we bucket those under a synthetic
# "<orchestrator>" agent so the wave rollup is complete. Per-agent wall-clock is
# derived from the min/max record timestamps for that agent (the documented
# subagent_tokens/duration_ms keys appear only inside human-readable summary STRINGS,
# not as structured fields, so timestamp-derived duration is the robust signal).
#
# USAGE
#   agent_telemetry.py TRANSCRIPT.jsonl [--json OUT.json] [--no-table]
#   agent_telemetry.py TRANSCRIPT.jsonl --prices prices.json
#   agent_telemetry.py T.jsonl --price-input 15 --price-cache-read 1.5 \
#                              --price-cache-write 18.75 --price-output 75
# (prices are $ per 1M tokens; with --price-input set the cache read/write default
#  to 0.1x / 1.25x of input per the documented multipliers unless overridden.)

from __future__ import annotations

import argparse
import datetime as _dt
import json
import sys
from dataclasses import dataclass, field
from typing import Any, Iterable, Iterator, Optional

ORCHESTRATOR_KEY = "<orchestrator>"
# Documented prompt-cache multipliers (research/agent-efficiency-tooling.md §1).
# Used only to DERIVE cache prices from --price-input when explicit cache prices are
# not supplied. They are ratios, not measurements, so they are safe to keep here.
DEFAULT_CACHE_READ_MULTIPLIER = 0.1
DEFAULT_CACHE_WRITE_MULTIPLIER = 1.25


# --------------------------------------------------------------------------------
# Token aggregation
# --------------------------------------------------------------------------------
@dataclass
class TokenTotals:
    """Summed token counters for one agent or one wave."""

    input_tokens: int = 0
    output_tokens: int = 0
    cache_read_input_tokens: int = 0
    cache_creation_input_tokens: int = 0
    # Split of cache writes by TTL (5-min sub-agent vs 1-hour main conversation).
    cache_creation_5m: int = 0
    cache_creation_1h: int = 0

    def add_usage(self, usage: dict) -> None:
        self.input_tokens += _int(usage.get("input_tokens"))
        self.output_tokens += _int(usage.get("output_tokens"))
        self.cache_read_input_tokens += _int(usage.get("cache_read_input_tokens"))
        self.cache_creation_input_tokens += _int(
            usage.get("cache_creation_input_tokens")
        )
        cc = usage.get("cache_creation")
        if isinstance(cc, dict):
            self.cache_creation_5m += _int(cc.get("ephemeral_5m_input_tokens"))
            self.cache_creation_1h += _int(cc.get("ephemeral_1h_input_tokens"))

    @property
    def total_input_side(self) -> int:
        """All input-side tokens billed (fresh input + cache read + cache write)."""
        return (
            self.input_tokens
            + self.cache_read_input_tokens
            + self.cache_creation_input_tokens
        )

    @property
    def cache_eligible(self) -> int:
        """Tokens that went through the cache (read hits + write misses)."""
        return self.cache_read_input_tokens + self.cache_creation_input_tokens

    @property
    def cache_hit_ratio(self) -> Optional[float]:
        """cache_read / (cache_read + cache_creation): the key efficiency signal.

        None when no cache-eligible tokens exist (ratio is undefined, not 0.0 —
        reporting 0.0 would imply a cold cache where there was simply no caching).
        """
        denom = self.cache_eligible
        if denom == 0:
            return None
        return self.cache_read_input_tokens / denom

    def as_dict(self) -> dict:
        return {
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "cache_creation_input_tokens": self.cache_creation_input_tokens,
            "cache_creation_5m_input_tokens": self.cache_creation_5m,
            "cache_creation_1h_input_tokens": self.cache_creation_1h,
            "total_input_side_tokens": self.total_input_side,
            "cache_hit_ratio": self.cache_hit_ratio,
        }


@dataclass
class AgentStats:
    """Per-agent rollup: identity, token totals, tool/turn counts, wall-clock."""

    agent_id: Optional[str]
    attribution_agent: Optional[str]
    is_sidechain: bool = False
    models: set = field(default_factory=set)
    tokens: TokenTotals = field(default_factory=TokenTotals)
    assistant_turns: int = 0
    tool_calls: int = 0
    first_ts: Optional[_dt.datetime] = None
    last_ts: Optional[_dt.datetime] = None

    @property
    def key(self) -> str:
        return self.agent_id or ORCHESTRATOR_KEY

    @property
    def label(self) -> str:
        if self.attribution_agent and self.agent_id:
            return f"{self.attribution_agent}:{self.agent_id[:8]}"
        if self.agent_id:
            return self.agent_id[:8]
        return ORCHESTRATOR_KEY

    @property
    def duration_seconds(self) -> Optional[float]:
        if self.first_ts is None or self.last_ts is None:
            return None
        return (self.last_ts - self.first_ts).total_seconds()

    def observe_ts(self, ts: Optional[_dt.datetime]) -> None:
        if ts is None:
            return
        if self.first_ts is None or ts < self.first_ts:
            self.first_ts = ts
        if self.last_ts is None or ts > self.last_ts:
            self.last_ts = ts

    def as_dict(self) -> dict:
        return {
            "agent_id": self.agent_id,
            "attribution_agent": self.attribution_agent,
            "label": self.label,
            "is_sidechain": self.is_sidechain,
            "models": sorted(self.models),
            "assistant_turns": self.assistant_turns,
            "tool_calls": self.tool_calls,
            "duration_seconds": self.duration_seconds,
            "tokens": self.tokens.as_dict(),
        }


@dataclass
class WaveReport:
    """The full per-transcript report: per-agent stats + a session rollup."""

    session_ids: list = field(default_factory=list)
    agents: dict = field(default_factory=dict)  # key -> AgentStats
    records_seen: int = 0
    records_parsed: int = 0

    def agent_for(
        self,
        agent_id: Optional[str],
        attribution_agent: Optional[str],
        is_sidechain: bool,
    ) -> AgentStats:
        key = agent_id or ORCHESTRATOR_KEY
        st = self.agents.get(key)
        if st is None:
            st = AgentStats(
                agent_id=agent_id,
                attribution_agent=attribution_agent,
                is_sidechain=is_sidechain,
            )
            self.agents[key] = st
        # Backfill attribution if a later record carries it but the first did not.
        if attribution_agent and not st.attribution_agent:
            st.attribution_agent = attribution_agent
        if is_sidechain:
            st.is_sidechain = True
        return st

    @property
    def rollup(self) -> TokenTotals:
        total = TokenTotals()
        for st in self.agents.values():
            t = st.tokens
            total.input_tokens += t.input_tokens
            total.output_tokens += t.output_tokens
            total.cache_read_input_tokens += t.cache_read_input_tokens
            total.cache_creation_input_tokens += t.cache_creation_input_tokens
            total.cache_creation_5m += t.cache_creation_5m
            total.cache_creation_1h += t.cache_creation_1h
        return total

    @property
    def wave_duration_seconds(self) -> Optional[float]:
        firsts = [st.first_ts for st in self.agents.values() if st.first_ts]
        lasts = [st.last_ts for st in self.agents.values() if st.last_ts]
        if not firsts or not lasts:
            return None
        return (max(lasts) - min(firsts)).total_seconds()

    def as_dict(self) -> dict:
        agents_sorted = sorted(
            self.agents.values(),
            key=lambda s: s.tokens.total_input_side,
            reverse=True,
        )
        return {
            "session_ids": self.session_ids,
            "records_seen": self.records_seen,
            "records_parsed": self.records_parsed,
            "agent_count": len(self.agents),
            "subagent_count": sum(
                1 for s in self.agents.values() if s.is_sidechain
            ),
            "wave_duration_seconds": self.wave_duration_seconds,
            "rollup": self.rollup.as_dict(),
            "agents": [s.as_dict() for s in agents_sorted],
        }


# --------------------------------------------------------------------------------
# Parsing
# --------------------------------------------------------------------------------
def _int(v: Any) -> int:
    try:
        return int(v)
    except (TypeError, ValueError):
        return 0


def _parse_ts(value: Any) -> Optional[_dt.datetime]:
    if not isinstance(value, str) or not value:
        return None
    s = value.replace("Z", "+00:00")
    try:
        dt = _dt.datetime.fromisoformat(s)
    except ValueError:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=_dt.timezone.utc)
    return dt


def iter_records(lines: Iterable[str]) -> Iterator[dict]:
    """Yield JSON object records from transcript lines, skipping non-objects.

    Transcript JSONL is robust to: blank lines, lines that decode to non-dicts
    (some tooling writes bare strings), and malformed lines. We skip those rather
    than abort so a partially-flushed live transcript still aggregates.
    """
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(obj, dict):
            yield obj


def _count_tool_calls(message: dict) -> int:
    content = message.get("content")
    if not isinstance(content, list):
        return 0
    return sum(
        1
        for block in content
        if isinstance(block, dict) and block.get("type") == "tool_use"
    )


def aggregate(records: Iterable[dict]) -> WaveReport:
    report = WaveReport()
    seen_sessions: list = []
    for rec in records:
        report.records_seen += 1
        sid = rec.get("sessionId")
        if sid and sid not in seen_sessions:
            seen_sessions.append(sid)
        if rec.get("type") != "assistant":
            continue
        message = rec.get("message")
        if not isinstance(message, dict):
            continue
        usage = message.get("usage")
        if not isinstance(usage, dict):
            continue
        report.records_parsed += 1
        st = report.agent_for(
            agent_id=rec.get("agentId"),
            attribution_agent=rec.get("attributionAgent"),
            is_sidechain=bool(rec.get("isSidechain")),
        )
        st.tokens.add_usage(usage)
        st.assistant_turns += 1
        st.tool_calls += _count_tool_calls(message)
        model = message.get("model")
        if model:
            st.models.add(model)
        st.observe_ts(_parse_ts(rec.get("timestamp")))
    report.session_ids = seen_sessions
    return report


def parse_transcript(path: str) -> WaveReport:
    with open(path, "r", encoding="utf-8") as fh:
        return aggregate(iter_records(fh))


# --------------------------------------------------------------------------------
# Cost (optional, price-table driven — never hard-coded vendor prices)
# --------------------------------------------------------------------------------
@dataclass
class PriceTable:
    """$ per 1,000,000 tokens. Cache read/write derive from input if unset."""

    input: float
    output: float
    cache_read: float
    cache_write: float

    @classmethod
    def from_args(cls, args: argparse.Namespace) -> Optional["PriceTable"]:
        if args.prices:
            with open(args.prices, "r", encoding="utf-8") as fh:
                data = json.load(fh)
            return cls(
                input=float(data["input"]),
                output=float(data["output"]),
                cache_read=float(
                    data.get(
                        "cache_read",
                        float(data["input"]) * DEFAULT_CACHE_READ_MULTIPLIER,
                    )
                ),
                cache_write=float(
                    data.get(
                        "cache_write",
                        float(data["input"]) * DEFAULT_CACHE_WRITE_MULTIPLIER,
                    )
                ),
            )
        if args.price_input is None:
            return None
        inp = float(args.price_input)
        return cls(
            input=inp,
            output=float(args.price_output if args.price_output is not None else 0.0),
            cache_read=float(
                args.price_cache_read
                if args.price_cache_read is not None
                else inp * DEFAULT_CACHE_READ_MULTIPLIER
            ),
            cache_write=float(
                args.price_cache_write
                if args.price_cache_write is not None
                else inp * DEFAULT_CACHE_WRITE_MULTIPLIER
            ),
        )

    def cost_for(self, t: TokenTotals) -> float:
        per = 1_000_000.0
        return (
            t.input_tokens * self.input
            + t.output_tokens * self.output
            + t.cache_read_input_tokens * self.cache_read
            + t.cache_creation_input_tokens * self.cache_write
        ) / per


# --------------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------------
def _fmt_int(n: int) -> str:
    return f"{n:,}"


def _fmt_ratio(r: Optional[float]) -> str:
    return "n/a" if r is None else f"{r * 100:5.1f}%"


def _fmt_dur(s: Optional[float]) -> str:
    if s is None:
        return "n/a"
    if s < 60:
        return f"{s:.0f}s"
    m, sec = divmod(int(s), 60)
    if m < 60:
        return f"{m}m{sec:02d}s"
    h, m = divmod(m, 60)
    return f"{h}h{m:02d}m"


def _fmt_cost(c: Optional[float]) -> str:
    return "n/a" if c is None else f"${c:,.4f}"


def render_table(report: WaveReport, prices: Optional[PriceTable] = None) -> str:
    """Human-readable per-agent table + a wave rollup line. Deterministic order."""
    agents_sorted = sorted(
        report.agents.values(),
        key=lambda s: s.tokens.total_input_side,
        reverse=True,
    )
    header = [
        "AGENT",
        "TURNS",
        "TOOLS",
        "INPUT",
        "OUTPUT",
        "CACHE_RD",
        "CACHE_WR",
        "HIT%",
        "DUR",
    ]
    if prices:
        header.append("COST")
    rows = []
    for st in agents_sorted:
        t = st.tokens
        row = [
            st.label,
            _fmt_int(st.assistant_turns),
            _fmt_int(st.tool_calls),
            _fmt_int(t.input_tokens),
            _fmt_int(t.output_tokens),
            _fmt_int(t.cache_read_input_tokens),
            _fmt_int(t.cache_creation_input_tokens),
            _fmt_ratio(t.cache_hit_ratio),
            _fmt_dur(st.duration_seconds),
        ]
        if prices:
            row.append(_fmt_cost(prices.cost_for(t)))
        rows.append(row)

    roll = report.rollup
    total_row = [
        "TOTAL (wave)",
        _fmt_int(sum(s.assistant_turns for s in report.agents.values())),
        _fmt_int(sum(s.tool_calls for s in report.agents.values())),
        _fmt_int(roll.input_tokens),
        _fmt_int(roll.output_tokens),
        _fmt_int(roll.cache_read_input_tokens),
        _fmt_int(roll.cache_creation_input_tokens),
        _fmt_ratio(roll.cache_hit_ratio),
        _fmt_dur(report.wave_duration_seconds),
    ]
    if prices:
        total_row.append(_fmt_cost(prices.cost_for(roll)))

    all_rows = [header] + rows + [total_row]
    widths = [max(len(r[i]) for r in all_rows) for i in range(len(header))]

    def fmt_row(r: list) -> str:
        # First column left-justified (labels), the rest right-justified (numbers).
        cells = [r[0].ljust(widths[0])]
        cells += [r[i].rjust(widths[i]) for i in range(1, len(r))]
        return "  ".join(cells)

    sep = "  ".join("-" * w for w in widths)
    lines = [fmt_row(header), sep]
    lines += [fmt_row(r) for r in rows]
    lines.append(sep)
    lines.append(fmt_row(total_row))

    subagents = sum(1 for s in report.agents.values() if s.is_sidechain)
    summary = (
        f"sessions={len(report.session_ids)}  agents={len(report.agents)} "
        f"(subagents={subagents})  "
        f"assistant-turns-parsed={report.records_parsed}/{report.records_seen} records"
    )
    note = (
        "NOTE: work-box / session-local measurement — NON-CANONICAL. "
        "Do not bake these numbers into docs."
    )
    return "\n".join(lines) + "\n\n" + summary + "\n" + note


def build_json(report: WaveReport, prices: Optional[PriceTable]) -> dict:
    out = report.as_dict()
    out["non_canonical"] = True
    out["note"] = (
        "Work-box/session-local measurement. NON-CANONICAL — do not bake into docs."
    )
    if prices:
        out["prices_usd_per_mtok"] = {
            "input": prices.input,
            "output": prices.output,
            "cache_read": prices.cache_read,
            "cache_write": prices.cache_write,
        }
        out["rollup"]["estimated_cost_usd"] = prices.cost_for(report.rollup)
        # Attach per-agent cost using the same deterministic ordering as as_dict().
        agents_sorted = sorted(
            report.agents.values(),
            key=lambda s: s.tokens.total_input_side,
            reverse=True,
        )
        for entry, st in zip(out["agents"], agents_sorted):
            entry["estimated_cost_usd"] = prices.cost_for(st.tokens)
    return out


# --------------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------------
def build_arg_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description=(
            "Aggregate Claude Code transcript token telemetry per agent and per "
            "wave. Reports NON-CANONICAL session-local numbers at runtime only."
        )
    )
    p.add_argument("transcript", help="path to a transcript JSONL file")
    p.add_argument("--json", dest="json_out", help="write structured JSON to PATH")
    p.add_argument(
        "--no-table",
        action="store_true",
        help="suppress the human-readable table (useful with --json)",
    )
    p.add_argument(
        "--prices",
        help="JSON file with $/1M-token prices {input,output[,cache_read,cache_write]}",
    )
    p.add_argument("--price-input", type=float, help="$ per 1M input tokens")
    p.add_argument("--price-output", type=float, help="$ per 1M output tokens")
    p.add_argument(
        "--price-cache-read",
        type=float,
        help="$ per 1M cache-read tokens (default 0.1x input)",
    )
    p.add_argument(
        "--price-cache-write",
        type=float,
        help="$ per 1M cache-write tokens (default 1.25x input)",
    )
    return p


def main(argv: Optional[list] = None) -> int:
    args = build_arg_parser().parse_args(argv)
    try:
        report = parse_transcript(args.transcript)
    except FileNotFoundError:
        print(f"error: transcript not found: {args.transcript}", file=sys.stderr)
        return 2
    prices = PriceTable.from_args(args)

    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as fh:
            json.dump(build_json(report, prices), fh, indent=2, sort_keys=True)
            fh.write("\n")
    if not args.no_table:
        print(render_table(report, prices))
    if report.records_parsed == 0:
        print(
            "warning: no assistant/usage records found — is this a transcript JSONL?",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
