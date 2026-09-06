#!/usr/bin/env python3
"""[GPT-6] Extract a completed campaign without converting coarse rate grids into equivalence."""
import argparse
import json
from pathlib import Path
from campaign_analysis import analyze_campaign


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_directory", type=Path)
    parser.add_argument("--review", type=Path, help="Exact-source benchmark-method/accounting review; missing review leaves inference unreviewed")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = analyze_campaign(args.artifact_directory, args.review)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + "\n")
    print(json.dumps({"output": str(args.output), "integrity_complete": result["artifact_integrity"]["complete"],
                      "source_review": result["source_review"]["status"], "cells": len(result["cells"]),
                      "full_service_million_history_admitted": False}))


if __name__ == "__main__": main()
