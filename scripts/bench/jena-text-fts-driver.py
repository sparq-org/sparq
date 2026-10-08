#!/usr/bin/env python3
# sq-hmd7l.2 — jena-text side of the FTS same-box comparison
# (scripts/bench/fts-same-box.sh). Runs the PINNED text:query translations
# (bench/fts/queries-jena-text/*.rq.tmpl x pairs.tsv) against a Fuseki+jena-text
# endpoint and emits the same 3-column `<workload>\t<count|ERROR>\t<us|reason>` TSV
# contract the rest of the same-box harnesses use:
#
#   count = the TOTAL matching-doc count summed over the 200 pinned pairs (the value
#           the envelope cross-checks against sparq-text's search().len() sums —
#           recorded as-is, NEVER adjusted; a mismatch is a result, not a fixup).
#   us    = best-of-iters total wall time / number of pairs (µs per query), measured
#           over a persistent keep-alive HTTP connection (reconnect-on-close, like
#           http_sparql_adapter's profile keep-alive regime). HTTP round-trip time —
#           the envelope records the mode asymmetry vs sparq's in-process index.
#
# Per-workload wall-clock cap (`timeout_s`): on breach the workload degrades to an
# honest `ERROR\ttimeout` row and the driver moves on — never a fabricated number.
# A count that differs across iterations (a non-deterministic answer) is likewise an
# `ERROR\tnondeterministic-counts` row.
#
# Usage:
#   jena-text-fts-driver.py <endpoint> <queries_dir> <pairs_tsv> <iters> <timeout_s>
#
# Reuses the unit-tested SPARQL-results-JSON parser from
# scripts/bench-adapters/http_sparql_adapter.py (the registered jena-text adapter).
import http.client
import os
import re
import sys
import time

sys.path.insert(
    0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bench-adapters")
)
from http_sparql_adapter import parse_sparql_json, split_endpoint  # noqa: E402

# The translated workloads. near_slop2 is deliberately ABSENT: Lucene's '"a b"~N'
# proximity is transposition-tolerant (unordered within slop) while sparq-text's
# text:near is ordered-within-gap — different result SETS, so no honest column exists
# (bench/fts/queries-jena-text/README.md). The envelope records the absence.
WORKLOADS = ("and_terms", "or_terms", "prefix4", "phrase")

# Pairs are machine-generated stem+digits words; anything else would need Lucene/SPARQL
# escaping the pinned templates do not perform, so refuse it loudly.
TERM_RE = re.compile(r"^[a-z]+[0-9]+$")


def load_pairs(path):
    pairs = []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            a, b = line.split("\t")
            if not (TERM_RE.match(a) and TERM_RE.match(b)):
                raise ValueError("unescapable term pair: %r / %r" % (a, b))
            pairs.append((a, b))
    return pairs


def instantiate(template, a, b):
    return (
        template.replace("%%A%%", a)
        .replace("%%B%%", b)
        .replace("%%P4%%", a[: min(len(a), 4)])
    )


class KeepAliveClient:
    """One persistent connection, transparently reconnected on server close."""

    def __init__(self, endpoint, timeout):
        self.host, self.port, self.path = split_endpoint(endpoint)
        self.timeout = timeout
        self.conn = http.client.HTTPConnection(self.host, self.port, timeout=timeout)

    def query(self, sparql):
        import urllib.parse

        body = urllib.parse.urlencode({"query": sparql})
        headers = {
            "Accept": "application/sparql-results+json",
            "Content-Type": "application/x-www-form-urlencoded",
        }
        try:
            self.conn.request("POST", self.path, body=body, headers=headers)
            resp = self.conn.getresponse()
        except (
            http.client.NotConnected,
            http.client.RemoteDisconnected,
            BrokenPipeError,
            ConnectionResetError,
        ):
            self.conn.close()
            self.conn = http.client.HTTPConnection(
                self.host, self.port, timeout=self.timeout
            )
            self.conn.request("POST", self.path, body=body, headers=headers)
            resp = self.conn.getresponse()
        data = resp.read()
        if resp.status != 200:
            raise ValueError("HTTP %d: %s" % (resp.status, data[:200]))
        return data.decode("utf-8")

    def close(self):
        self.conn.close()


def run_workload(client, template, pairs, iters, timeout_s):
    """Returns (total_count, best_us_per_query) or raises TimeoutError/ValueError."""
    queries = [instantiate(template, a, b) for a, b in pairs]
    deadline = time.monotonic() + timeout_s
    totals = set()
    best_total_us = None
    for _ in range(max(1, iters)):
        t0 = time.perf_counter()
        total = 0
        for q in queries:
            if time.monotonic() > deadline:
                raise TimeoutError("timeout")
            parsed = parse_sparql_json(client.query(q))
            if parsed["count_value"] is None:
                raise ValueError("no-count-value")
            total += int(parsed["count_value"])
        total_us = (time.perf_counter() - t0) * 1e6
        totals.add(total)
        if best_total_us is None or total_us < best_total_us:
            best_total_us = total_us
    if len(totals) != 1:
        # A hit count that moves across identical reruns is a non-deterministic
        # answer — recorded as an error, never averaged away.
        raise ValueError("nondeterministic-counts")
    return totals.pop(), best_total_us / len(pairs)


def main(argv):
    if len(argv) != 6:
        print(__doc__ or "", file=sys.stderr)
        print(
            "usage: jena-text-fts-driver.py <endpoint> <queries_dir> <pairs_tsv>"
            " <iters> <timeout_s>",
            file=sys.stderr,
        )
        return 2
    endpoint, queries_dir, pairs_tsv = argv[1], argv[2], argv[3]
    iters, timeout_s = int(argv[4]), float(argv[5])

    pairs = load_pairs(pairs_tsv)
    client = KeepAliveClient(endpoint, timeout=min(timeout_s, 300.0))
    try:
        for wl in WORKLOADS:
            tmpl_path = os.path.join(queries_dir, "%s.rq.tmpl" % wl)
            with open(tmpl_path, encoding="utf-8") as fh:
                template = fh.read()
            try:
                count, us = run_workload(client, template, pairs, iters, timeout_s)
                print("%s\t%d\t%.1f" % (wl, count, us))
            except TimeoutError:
                print("%s\tERROR\ttimeout" % wl)
                print("[jena-text-fts] %s: timeout" % wl, file=sys.stderr)
            except Exception as e:  # transport/parse/shape error -> honest ERROR row
                reason = str(e).split("\n")[0][:120] or type(e).__name__
                print("%s\tERROR\t%s" % (wl, reason.replace("\t", " ")))
                print("[jena-text-fts] %s: %s" % (wl, reason), file=sys.stderr)
            sys.stdout.flush()
    finally:
        client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
