#!/usr/bin/env python3
# sq-eifd: HTTP-SPARQL adapter KIND.
#
# POST a SPARQL query to an HTTP endpoint -> parse the SPARQL 1.1 Query Results
# JSON -> count solutions / read the ASK boolean / extract a COUNT(*) value.
# Shared across Fuseki, Virtuoso (the core-SPARQL reference engines from
# research/BENCHMARKS.md) AND the existing QLever HTTP path (this is the reusable
# unit the per-dir bench/qlever-*/compare.py hand-rolls today; G3 in
# research/capability-benchmark-program.md §2).
#
# The PARSER (parse_sparql_json) is the value here and is unit-tested against a
# captured sample response WITHOUT a live server. The transport (do_query) is a
# thin urllib POST; standing up Fuseki/Virtuoso is gather-only on a Docker box.
#
# SPARQL-JSON shapes handled (per the W3C rec):
#   - SELECT : {"head":{"vars":[...]}, "results":{"bindings":[ {var:{...}}, ...]}}
#              -> solution count = len(bindings); a single-binding single-var row
#                 is treated as a COUNT(*) and its value extracted (the
#                 correctness cross-check QLever's compare.py does today).
#   - ASK    : {"head":{}, "boolean": true|false} -> count = 1 if true else 0,
#              boolean carried through.
#
# Output: `<engine>\t<count>\t<query_us>` TSV on stdout (the same 3-col contract
# the rest of the adapters + the ci-bench hook use). Non-zero exit only on a
# transport/parse ERROR.
# sq-7d3dj.34: --profile mode. The plain 3-col contract above is unchanged;
# --profile ADDITIONALLY measures TTFB (time to first response byte, i.e. status line +
# headers received) and runs the query in BOTH connection regimes:
#   keep-alive : one persistent http.client.HTTPConnection reused across iters (the
#                steady-state server-performance measure; iter 1 pays the TCP connect,
#                min-of-N discards it once any later iter reuses the socket).
#   fresh      : a NEW TCP connection per iter (cold-client regime; includes connect).
# Emitting BOTH answers the keep-alive-vs-fresh fairness question with data instead of a
# methodology argument. Profile stdout contract (6-col):
#   <engine>\t<count>\t<ka_best_us>\t<ka_ttfb_us>\t<fresh_best_us>\t<fresh_ttfb_us>
# best_us and ttfb_us are independent per-mode minima over iters (each is the honest
# floor of its own distribution). A server that closes the connection mid-series is
# transparently reconnected (recorded in the --json sidecar as keepalive_reconnects).
import http.client
import json
import sys
import time
import urllib.parse
import urllib.request


def parse_sparql_json(text):
    """Reduce a SPARQL-results-JSON document to a comparable count.

    Returns {"count": int, "boolean": bool|None, "count_value": str|None}:
      count       : len(bindings) for SELECT, or 1/0 for ASK true/false.
      boolean     : the ASK boolean (None for SELECT).
      count_value : the scalar of a single-row single-var SELECT (a COUNT(*) wrap),
                    else None — the value to cross-check against sparq's count.
    Raises ValueError on a document that is neither a SELECT nor an ASK result."""
    obj = text if isinstance(text, dict) else json.loads(text)
    if "boolean" in obj:
        # Per the SPARQL 1.1 Query Results JSON rec the ASK "boolean"
        # member is a JSON boolean. Reject anything else (e.g. the string "false",
        # which bool() would wrongly coerce to True) as an invalid document rather
        # than silently mis-counting it.
        raw = obj["boolean"]
        if not isinstance(raw, bool):
            raise ValueError(
                "SPARQL-JSON 'boolean' is not a JSON boolean: %r" % (raw,)
            )
        return {"count": 1 if raw else 0, "boolean": raw, "count_value": None}
    results = obj.get("results")
    if results is None or "bindings" not in results:
        raise ValueError("not a SPARQL SELECT/ASK results document")
    bindings = results["bindings"]
    count_value = None
    if len(bindings) == 1 and len(bindings[0]) == 1:
        only = next(iter(bindings[0].values()))
        count_value = only.get("value")
    return {"count": len(bindings), "boolean": None, "count_value": count_value}


def do_query(endpoint, query, accept="application/sparql-results+json", timeout=300):
    """POST a SPARQL query (application/x-www-form-urlencoded) -> response bytes."""
    data = urllib.parse.urlencode({"query": query}).encode()
    req = urllib.request.Request(
        endpoint, data=data, headers={"Accept": accept}
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read().decode("utf-8")


def run_http_sparql(endpoint, query, engine="engine", iters=1, timeout=300):
    """Run a query `iters` times (min wall-clock), parse the JSON result.

    Returns {"engine","count","boolean","count_value","query_us"}.
    Raises on transport/parse error."""
    best_us = None
    parsed = None
    for _ in range(max(1, iters)):
        t0 = time.perf_counter()
        body = do_query(endpoint, query, timeout=timeout)
        dt_us = (time.perf_counter() - t0) * 1e6
        parsed = parse_sparql_json(body)
        if best_us is None or dt_us < best_us:
            best_us = dt_us
    return {
        "engine": engine,
        "count": parsed["count"],
        "boolean": parsed["boolean"],
        "count_value": parsed["count_value"],
        "query_us": int(round(best_us)),
    }


def split_endpoint(endpoint):
    """Split an http(s) endpoint URL into (host, port, path-with-query).

    Pure function (offline-testable). Raises ValueError on a non-http(s) scheme
    or a missing host."""
    parts = urllib.parse.urlsplit(endpoint)
    if parts.scheme not in ("http", "https"):
        raise ValueError("profile mode needs an http(s) endpoint: %r" % (endpoint,))
    if not parts.hostname:
        raise ValueError("endpoint has no host: %r" % (endpoint,))
    port = parts.port or (443 if parts.scheme == "https" else 80)
    path = parts.path or "/"
    if parts.query:
        path += "?" + parts.query
    return parts.hostname, port, path


def format_profile_tsv(engine, count, ka, fresh):
    """Format the 6-col profile stdout line from two {'best_us','ttfb_us'} dicts.

    Pure function (offline-testable)."""
    return "%s\t%d\t%d\t%d\t%d\t%d" % (
        engine,
        count,
        ka["best_us"],
        ka["ttfb_us"],
        fresh["best_us"],
        fresh["ttfb_us"],
    )


def _profile_once(conn, path, body, headers):
    """One timed request on an (possibly already-connected) HTTPConnection.

    Returns (ttfb_us, full_us, response_text). TTFB = first response byte
    received (status line + headers parsed by getresponse())."""
    t0 = time.perf_counter()
    conn.request("POST", path, body=body, headers=headers)
    resp = conn.getresponse()
    ttfb_us = (time.perf_counter() - t0) * 1e6
    data = resp.read()
    full_us = (time.perf_counter() - t0) * 1e6
    if resp.status != 200:
        raise ValueError("HTTP %d: %s" % (resp.status, data[:200]))
    return ttfb_us, full_us, data.decode("utf-8")


def profile_http_sparql(
    endpoint, query, engine="engine", iters=1, timeout=300,
    accept="application/sparql-results+json",
):
    """Measure the query in keep-alive AND fresh-connect regimes with TTFB.

    Returns {"engine","count","boolean","count_value",
             "keepalive":{"best_us","ttfb_us","reconnects"},
             "fresh":{"best_us","ttfb_us"}}.
    The parsed COUNT is asserted identical across every iteration of both
    regimes (a drift would mean a non-deterministic answer — fail loudly)."""
    host, port, path = split_endpoint(endpoint)
    body = urllib.parse.urlencode({"query": query})
    headers = {
        "Accept": accept,
        "Content-Type": "application/x-www-form-urlencoded",
    }
    counts = set()
    parsed = None

    # --- keep-alive: ONE connection reused across iters (reconnect on server close) ---
    ka_best = ka_ttfb = None
    reconnects = 0
    conn = http.client.HTTPConnection(host, port, timeout=timeout)
    try:
        for _ in range(max(1, iters)):
            try:
                ttfb_us, full_us, text = _profile_once(conn, path, body, headers)
            except (http.client.NotConnected, http.client.RemoteDisconnected,
                    BrokenPipeError, ConnectionResetError):
                # server closed the persistent connection: reconnect once for this iter
                conn.close()
                conn = http.client.HTTPConnection(host, port, timeout=timeout)
                reconnects += 1
                ttfb_us, full_us, text = _profile_once(conn, path, body, headers)
            parsed = parse_sparql_json(text)
            counts.add(parsed["count"])
            ka_best = full_us if ka_best is None else min(ka_best, full_us)
            ka_ttfb = ttfb_us if ka_ttfb is None else min(ka_ttfb, ttfb_us)
    finally:
        conn.close()

    # --- fresh: a NEW TCP connection per iter (connect cost included) ---
    fr_best = fr_ttfb = None
    for _ in range(max(1, iters)):
        conn = http.client.HTTPConnection(host, port, timeout=timeout)
        try:
            ttfb_us, full_us, text = _profile_once(conn, path, body, headers)
        finally:
            conn.close()
        parsed = parse_sparql_json(text)
        counts.add(parsed["count"])
        fr_best = full_us if fr_best is None else min(fr_best, full_us)
        fr_ttfb = ttfb_us if fr_ttfb is None else min(fr_ttfb, ttfb_us)

    if len(counts) != 1:
        raise ValueError("count drifted across iterations: %s" % sorted(counts))
    return {
        "engine": engine,
        "count": parsed["count"],
        "boolean": parsed["boolean"],
        "count_value": parsed["count_value"],
        "keepalive": {
            "best_us": int(round(ka_best)),
            "ttfb_us": int(round(ka_ttfb)),
            "reconnects": reconnects,
        },
        "fresh": {"best_us": int(round(fr_best)), "ttfb_us": int(round(fr_ttfb))},
    }


def main(argv):
    """CLI: http_sparql_adapter.py --endpoint URL (--query Q | --query-file F)
            [--engine NAME] [--iters N] [--json] [--profile]
       OR : --parse-only <json-file>   (offline: just run the parser, for tests)"""
    endpoint = query = None
    engine = "engine"
    iters = 1
    want_json = False
    want_profile = False
    parse_only = None
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--endpoint":
            i += 1
            endpoint = argv[i]
        elif a == "--query":
            i += 1
            query = argv[i]
        elif a == "--query-file":
            i += 1
            with open(argv[i], "r", encoding="utf-8") as fh:
                query = fh.read()
        elif a == "--engine":
            i += 1
            engine = argv[i]
        elif a == "--iters":
            i += 1
            iters = int(argv[i])
        elif a == "--parse-only":
            i += 1
            parse_only = argv[i]
        elif a == "--json":
            want_json = True
        elif a == "--profile":
            want_profile = True
        else:
            sys.stderr.write("http_sparql_adapter: unknown arg %s\n" % a)
            return 2
        i += 1

    if parse_only is not None:
        with open(parse_only, "r", encoding="utf-8") as fh:
            res = parse_sparql_json(fh.read())
        json.dump(res, sys.stdout)
        sys.stdout.write("\n")
        return 0

    if not (endpoint and query):
        sys.stderr.write(
            "usage: http_sparql_adapter.py --endpoint URL (--query Q|--query-file F) "
            "[--engine NAME] [--iters N] [--json] [--profile]   |   "
            "--parse-only <json-file>\n"
        )
        return 2
    if want_profile:
        try:
            res = profile_http_sparql(endpoint, query, engine=engine, iters=iters)
        except Exception as e:  # noqa: BLE001 — adapter boundary
            sys.stderr.write("http_sparql_adapter: %s\n" % e)
            return 1
        sys.stdout.write(
            format_profile_tsv(res["engine"], res["count"], res["keepalive"], res["fresh"])
            + "\n"
        )
        if want_json:
            sys.stderr.write(json.dumps(res) + "\n")
        return 0
    try:
        res = run_http_sparql(endpoint, query, engine=engine, iters=iters)
    except Exception as e:  # noqa: BLE001 — adapter boundary
        sys.stderr.write("http_sparql_adapter: %s\n" % e)
        return 1
    sys.stdout.write("%s\t%d\t%d\n" % (res["engine"], res["count"], res["query_us"]))
    if want_json:
        sys.stderr.write(json.dumps(res) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
