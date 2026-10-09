#!/usr/bin/env python3
# sq-hmd7l.15 — Titanium JSON-LD (Java) adapter for the bench/jsonld
# suite (registered in bench/competitors.json, id: titanium-json-ld). GATHER-ONLY:
# the jars are NOT committed dependencies — download at gather time and point
# TITANIUM_CP at ALL FIVE (titanium 1.7.x splits its RDF primitives into
# separate artifacts; language-tag parsing needs titanium-rdf-n-quads), e.g.
#   TITANIUM_CP="titanium-json-ld-1.7.0.jar:jakarta.json-api-2.1.3.jar:\
#                parsson-1.1.7.jar:titanium-rdf-api-1.0.0.jar:titanium-rdf-n-quads-1.0.2.jar"
# (Maven Central: com.apicatalog:titanium-json-ld + :titanium-rdf-api +
#  :titanium-rdf-n-quads, jakarta.json:jakarta.json-api, org.eclipse.parsson:parsson.
#  Apache-2.0/EPL-2.0 — numbers are publishable.)
#
# Contract (shared with jsonld_adapter.mjs / the sparq bench_jsonld example):
#   --engine titanium --op expand|flatten|compact|tordf --input doc.jsonld
#   [--context ctx.jsonld] [--iters N] [--warmup W] [--out file]
# Emits the operation OUTPUT to --out (for the harness's output-equality gate —
# run BEFORE any timing row is trusted) and one JSON envelope line on stdout.
# The timing loop runs IN-JVM (one process, warmup first) so JVM startup is
# excluded; the task boundary matches the peers: in-memory JSON text ->
# operation result, JSON parse inside the loop, serialization outside.
# All timings are advisory wall-clock — never canonical numbers.
import argparse
import json
import os
import subprocess
import sys
import tempfile

# The in-JVM runner. Compiled once per invocation with javac (fast; the source
# is tiny). Fixtures are self-contained; a remote-context fetch would surface
# as an output-equality failure in the harness gate.
JAVA_SRC = r"""
import com.apicatalog.jsonld.JsonLd;
import com.apicatalog.jsonld.document.Document;
import com.apicatalog.jsonld.document.JsonDocument;
import com.apicatalog.rdf.RdfDataset;
import com.apicatalog.rdf.io.nquad.NQuadsWriter;
import jakarta.json.JsonStructure;
import java.io.StringReader;
import java.io.StringWriter;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;

public final class TitaniumBench {
    static String text;
    static String ctxText;
    static final String BASE = "https://w3id.org/sparq/bench/jsonld/";

    static String runOnce(String op) throws Exception {
        // JSON parse inside the loop — the shared task boundary.
        Document doc = JsonDocument.of(new StringReader(text));
        switch (op) {
            case "expand": {
                JsonStructure out = JsonLd.expand(doc).base(URI.create(BASE)).get();
                return out.toString();
            }
            case "flatten": {
                JsonStructure out = JsonLd.flatten(doc).base(URI.create(BASE)).get();
                return out.toString();
            }
            case "compact": {
                Document ctx = JsonDocument.of(new StringReader(ctxText));
                JsonStructure out = JsonLd.compact(doc, ctx).base(URI.create(BASE)).get();
                return out.toString();
            }
            case "tordf": {
                RdfDataset ds = JsonLd.toRdf(doc).base(URI.create(BASE)).get();
                StringWriter w = new StringWriter();
                new NQuadsWriter(w).write(ds);
                return w.toString();
            }
            default: throw new IllegalArgumentException("unknown op " + op);
        }
    }

    public static void main(String[] argv) throws Exception {
        String op = argv[0];
        text = new String(Files.readAllBytes(Paths.get(argv[1])), StandardCharsets.UTF_8);
        int iters = Integer.parseInt(argv[2]);
        int warmup = Integer.parseInt(argv[3]);
        String outFile = argv[4];
        if (argv.length > 5) {
            ctxText = new String(Files.readAllBytes(Paths.get(argv[5])), StandardCharsets.UTF_8);
        }
        String result = runOnce(op); // correctness output for the equality gate
        if (!outFile.equals("-")) {
            Files.write(Paths.get(outFile), result.getBytes(StandardCharsets.UTF_8));
        }
        for (int i = 0; i < warmup; i++) runOnce(op);
        long t0 = System.nanoTime();
        for (int i = 0; i < iters; i++) runOnce(op);
        double usPerOp = (System.nanoTime() - t0) / 1000.0 / iters;
        // One machine-readable line the python wrapper picks up.
        System.out.println("TITANIUM_US_PER_OP\t" + usPerOp);
    }
}
"""


def main() -> int:
    ap = argparse.ArgumentParser(description="Titanium JSON-LD bench adapter (gather-only)")
    ap.add_argument("--engine", default="titanium", choices=["titanium"])
    ap.add_argument("--op", required=True, choices=["expand", "flatten", "compact", "tordf"])
    ap.add_argument("--input", required=True)
    ap.add_argument("--context")
    ap.add_argument("--iters", type=int, default=20)
    ap.add_argument("--warmup", type=int, default=3)
    ap.add_argument("--out")
    args = ap.parse_args()

    cp = os.environ.get("TITANIUM_CP")
    if not cp:
        print("jsonld_adapter.py: TITANIUM_CP not set (gather-only adapter; see header)", file=sys.stderr)
        return 3
    if args.op == "compact" and not args.context:
        print("jsonld_adapter.py: --op compact needs --context", file=sys.stderr)
        return 2

    with tempfile.TemporaryDirectory(prefix="titanium-bench-") as tmp:
        src = os.path.join(tmp, "TitaniumBench.java")
        with open(src, "w", encoding="utf-8") as f:
            f.write(JAVA_SRC)
        subprocess.run(["javac", "-cp", cp, "-d", tmp, src], check=True)
        cmd = [
            "java", "-cp", "{}:{}".format(tmp, cp), "TitaniumBench",
            args.op, args.input, str(max(1, args.iters)), str(max(0, args.warmup)),
            args.out or "-",
        ]
        if args.context:
            cmd.append(args.context)
        run = subprocess.run(cmd, check=True, capture_output=True, text=True)

    us_per_op = None
    for line in run.stdout.splitlines():
        if line.startswith("TITANIUM_US_PER_OP\t"):
            us_per_op = float(line.split("\t", 1)[1])
    if us_per_op is None:
        print("jsonld_adapter.py: runner emitted no timing line", file=sys.stderr)
        return 1

    size = os.path.getsize(args.input)
    docs_per_s = 1e6 / us_per_op
    # Pin the exact artifact by classpath jar name (the version provenance).
    jars = [os.path.basename(p) for p in cp.split(":") if "titanium" in os.path.basename(p)]
    print(json.dumps({
        "engine": "titanium-json-ld",
        "engine_version": jars[0] if jars else "unknown (set TITANIUM_CP to the pinned jar)",
        "op": args.op,
        "input": args.input,
        "bytes": size,
        "iters": args.iters,
        "us_per_op": round(us_per_op, 1),
        "docs_per_s": round(docs_per_s, 1),
        "mb_per_s": round(size / 1e6 * docs_per_s, 2),
    }))
    return 0


if __name__ == "__main__":
    sys.exit(main())
