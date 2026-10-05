#!/usr/bin/env python3
# [OPUS-4.8] sq-ve5dy / sq-zbyo7 (epic sq-2m6zm). 🤖 SPARQ agent — REAL-transcript
# effective-token miner for the 3-arm PKG token A/B. Written while Fable unavailable;
# flag for re-review when Fable returns.
#
# WHAT THIS IS — and how it differs from tokens.py
# --------------------------------------------------------------------------------
# tokens.py estimates a *read payload* via a documented char/token PROXY (the #1078
# PoC). THIS miner does the real thing: it reads the actual per-(task,arm) sub-agent
# transcripts produced by the two pkg-token-ab workflow runs and sums the
# cache-discounted EFFECTIVE INPUT TOKENS straight from each assistant message's
# `message.usage` block. NO count_tokens API, NO char proxy — these are the tokens
# the model actually consumed.
#
#   effective_input = 1.0*input + 0.1*cache_read + 1.25*cache_creation
#
# (the canonical §5.1 metrics_row.py multipliers). Each transcript is one fresh
# sub-agent that answered exactly one (task, arm); its first line carries the tag
#   [ABM task=<id> arm=<A|B|C>]
# so the miner can attribute the run. Output is a JSON list of per-transcript rows
# the 3-arm analyzer (analyze3.py) consumes.
#
# Usage:
#   tokens_real.py <transcript-dir> [out.json]
#
# <transcript-dir> contains one `agent-*.jsonl` Claude Code transcript per sub-agent.

from __future__ import annotations

import glob
import json
import os
import re
import sys

# The §5.1 cache-discount multipliers (same as metrics_row.py / tokens.py). Kept as
# literals here so the miner runs standalone on a transcript dir with no repo on the
# path; the values are asserted-equal to the canonical ones by the harness test.
EFF_FRESH, EFF_CACHE_READ, EFF_CACHE_WRITE = 1.0, 0.1, 1.25

# Each fresh sub-agent's brief opens with this tag so we can attribute its transcript
# to one (task, arm). The orchestrator that fans out the runs writes it verbatim.
# Arms: A/B/C are the read-docs vs pkg-query 3-arm A/B (sq-ve5dy); P0..P3 are the
# GenAI-KB Phase 7 provenance-capability arms (sq-2489d.6 — inert-PKG baseline P0 vs
# one capability each, see PREREG-PROVENANCE.md). [OPUS-5]
TAG = re.compile(r"\[ABM task=(\S+) arm=([ABC]|P[0-3])\]")


def mine(transcript_dir: str) -> list[dict]:
    """Mine every `agent-*.jsonl` transcript in `transcript_dir` into a per-run row."""
    rows: list[dict] = []
    for path in sorted(glob.glob(os.path.join(transcript_dir, "agent-*.jsonl"))):
        task = arm = None
        eff = 0.0
        fresh = cache_read = cache_write = out_tok = 0
        n_assist = 0
        with open(path, encoding="utf-8") as fh:
            for line in fh:
                line = line.rstrip("\n")
                if not line:
                    continue
                # The tag may appear anywhere in the first user turn; scan until found.
                if task is None:
                    m = TAG.search(line)
                    if m:
                        task, arm = m.group(1), m.group(2)
                try:
                    obj = json.loads(line)
                except (json.JSONDecodeError, ValueError):
                    continue
                if obj.get("type") != "assistant":
                    continue
                usage = (obj.get("message") or {}).get("usage") or {}
                fi = int(usage.get("input_tokens") or 0)
                rd = int(usage.get("cache_read_input_tokens") or 0)
                wr = int(usage.get("cache_creation_input_tokens") or 0)
                ot = int(usage.get("output_tokens") or 0)
                if fi or rd or wr:
                    n_assist += 1
                    fresh += fi
                    cache_read += rd
                    cache_write += wr
                    out_tok += ot
                    eff += EFF_FRESH * fi + EFF_CACHE_READ * rd + EFF_CACHE_WRITE * wr
        rows.append(
            {
                "agent_file": os.path.basename(path),
                "task": task,
                "arm": arm,
                "eff_input_tokens": round(eff, 1),
                "fresh": fresh,
                "cache_read": cache_read,
                "cache_write": cache_write,
                "output_tokens": out_tok,
                "assistant_turns": n_assist,
            }
        )
    return rows


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("usage: tokens_real.py <transcript-dir> [out.json]", file=sys.stderr)
        return 2
    transcript_dir = argv[1]
    out = argv[2] if len(argv) > 2 else "tokens_real_out.json"
    rows = mine(transcript_dir)
    tagged = [r for r in rows if r["task"]]
    print(f"transcripts: {len(rows)} | ABM-tagged: {len(tagged)}")
    for r in rows[:6]:
        print(
            f"  {r['agent_file'][:24]} task={r['task']} arm={r['arm']} "
            f"eff={r['eff_input_tokens']} "
            f"(fresh={r['fresh']} cr={r['cache_read']} cw={r['cache_write']}) "
            f"turns={r['assistant_turns']}"
        )
    with open(out, "w", encoding="utf-8") as fh:
        json.dump(rows, fh, indent=2)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
