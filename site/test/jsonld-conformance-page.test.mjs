import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import { jsonLdConformanceLanes, jsonLdEarlReport } from "../src/data/jsonld-conformance.ts";

const root = new URL("../", import.meta.url);
const pageUrl = new URL("src/app/assurance/jsonld-conformance/page.tsx", root);

test("JSON-LD scoreboard separates processor API and store ingest measurements", async () => {
  const page = await readFile(pageUrl, "utf8");
  assert.deepEqual(
    jsonLdConformanceLanes.map(({ id, floor, total }) => [id, floor, total]),
    [
      ["toRdf", 454, 467],
      ["toRdf-ingest", 429, 467],
      ["expand", 381, 385],
      ["flatten", 55, 58],
      ["compact", 244, 246],
      ["frame", 92, 92],
      ["fromRdf", 52, 53],
      ["html", 49, 50],
      ["remote-doc", 18, 18],
    ],
  );
  assert.deepEqual(jsonLdEarlReport, {
    passed: 1295, failed: 1, total: 1296, toRdfPassed: 448, toRdfTotal: 448,
  });
  assert.equal(jsonLdConformanceLanes[0].label, "Processor API: toRdf");
  assert.equal(jsonLdConformanceLanes[0].component, "sparq-jsonld (1.1); oxjsonld (1.0-only compatibility)");
  assert.equal(jsonLdConformanceLanes[1].label, "Store ingest: toRdf");
  assert.equal(jsonLdConformanceLanes[1].component, "sparq-core Graph over oxjsonld");
  assert.equal(jsonLdConformanceLanes[1].failureCeiling, 38);
  assert.match(page, /jsonLdConformanceLanes\.map/);
  assert.match(page, /\{lane\.label\}/);
  assert.match(page, /\{lane\.component\}/);
  assert.match(page, /\{lane\.floor\}\/\{lane\.total\}/);
  assert.match(page, /lane\.id === "toRdf-ingest" &&/);
  assert.match(page, /Excluded from EARL; failure ceiling \{lane\.failureCeiling\}/);
});

test("page attributes EARL to the processor and explains the pending ingest switch", async () => {
  const page = await readFile(pageUrl, "utf8");
  assert.match(page, /EARL report measures the <strong>sparq-jsonld processor API<\/strong>/);
  assert.match(page, /\{jsonLdEarlReport\.passed\}\/\{jsonLdEarlReport\.total\}/);
  assert.match(page, /\{jsonLdEarlReport\.toRdfPassed\}\/\{jsonLdEarlReport\.toRdfTotal\}/);
  assert.match(page, /It does not measure store ingest/);
  assert.match(page, /Store ingest uses <strong>sparq-core Graph over oxjsonld<\/strong>/);
  assert.match(page, /href="https:\/\/github\.com\/sparq-org\/sparq\/issues\/7342"/);
  assert.match(page, /Until that switch lands, both paths have separate results below/);
  assert.match(page, /Measured floors, not conformance claims/);
  assert.match(page, /A lane below its total is not claimed conformant/);
  assert.match(page, /toRdf\s+runner uses sparq-jsonld for 1\.1 cases and oxjsonld for 1\.0-only compatibility cases/);
  assert.match(page, /EARL omits 1\.0-only entries/);
  assert.match(page, /runs all 467 toRdf entries without skips/);
});

test("site snapshots match every lib-side pass floor and the ingest failure ceiling", async () => {
  const modules = {
    toRdf: "to_rdf", "toRdf-ingest": "to_rdf_ingest", expand: "expand",
    flatten: "flatten", compact: "compact", frame: "frame", fromRdf: "from_rdf",
    html: "html", "remote-doc": "remote_doc",
  };
  await Promise.all(jsonLdConformanceLanes.map(async (lane) => {
    const source = await readFile(new URL(`../crates/sparq-conformance/src/floors/${modules[lane.id]}.rs`, root), "utf8");
    const floor = source.match(/^pub const FLOOR: usize = (\d+);/m);
    assert.ok(floor, `missing ${lane.id} floor`);
    assert.equal(lane.floor, Number(floor[1]), `${lane.id} site floor drifted`);
    if (lane.id === "toRdf-ingest") {
      const ceiling = source.match(/^pub const FAIL_CEILING: usize = (\d+);/m);
      assert.ok(ceiling, "missing ingest failure ceiling");
      assert.equal(lane.failureCeiling, Number(ceiling[1]), "ingest ceiling drifted");
    }
  }));
});
