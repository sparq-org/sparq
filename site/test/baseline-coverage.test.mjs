// [OPUS-4.8] sq-vw3ax.12 — unit tests for the competitor-baseline coverage derivation
// (src/lib/baseline-coverage.ts deriveCoverage). The load-bearing honesty invariant: a cell
// is "measured" ONLY when the engine produced a real timing in a committed same-box gather; a
// missing engine/suite is "pending" (never fabricated), and an attempted-but-failed engine is
// "failed" — unless it also succeeded in another mode, in which case measured wins.
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  deriveCoverage,
  COVERAGE_ENGINES,
  COVERAGE_SUITES,
} from "../src/lib/baseline-coverage.ts";

// A tiny same-box comparison factory. `rows` maps engineId -> timing (number|null).
function comparison(suite, engines, rows, { canonical = true } = {}) {
  return {
    suite,
    scale: "test",
    iters: 5,
    git_commit: "abc123",
    gathered_at_utc: "2026-07-19T00:00:00Z",
    source: "test",
    canonical,
    env: { host_class: "test-box", quiet_box: true },
    engines,
    rows: rows.map((values, i) => ({
      query: `q0${i + 1}`,
      unit: "us",
      rows: 1,
      values,
      count_match: true,
    })),
  };
}

const CLI = "CLI in-process";
const HTTP = "HTTP SPARQL adapter";

test("an engine with a real timing is 'measured'; a missing engine is 'pending'", () => {
  const comps = [
    comparison(
      "sp2b",
      [
        { id: "sparq", label: "sparq", mode: CLI, status: "ok" },
        { id: "oxigraph", label: "Oxigraph", mode: CLI, status: "ok" },
      ],
      [{ sparq: 100, oxigraph: 200 }],
    ),
  ];
  const m = deriveCoverage(comps);
  assert.equal(m.cells["sp2b"]["oxigraph"].state, "measured");
  assert.equal(m.cells["sp2b"]["oxigraph"].canonical, true);
  assert.equal(m.cells["sp2b"]["oxigraph"].mode, "cli");
  // an engine never gathered on any suite is pending everywhere
  assert.equal(m.cells["sp2b"]["blazegraph"].state, "pending");
  assert.equal(m.cells["bsbm"]["oxigraph"].state, "pending");
});

test("a suite with no gather at all is entirely pending (LUBM / BSBM today)", () => {
  const m = deriveCoverage([]);
  for (const s of COVERAGE_SUITES) {
    for (const e of COVERAGE_ENGINES) {
      assert.equal(m.cells[s.id][e.id].state, "pending");
    }
  }
  assert.equal(m.measuredCount, 0);
  assert.equal(m.totalCount, COVERAGE_SUITES.length * COVERAGE_ENGINES.length);
});

test("an attempted engine that produced no timing and failed is 'failed'", () => {
  const comps = [
    comparison(
      "sp2b",
      [
        { id: "sparq", label: "sparq", mode: CLI, status: "ok" },
        {
          id: "fuseki",
          label: "Apache Jena Fuseki",
          mode: HTTP,
          status: "failed",
          failure: "load timeout",
        },
      ],
      [{ sparq: 100, fuseki: null }],
    ),
  ];
  const m = deriveCoverage(comps);
  const cell = m.cells["sp2b"]["fuseki"];
  assert.equal(cell.state, "failed");
  assert.equal(cell.note, "load timeout");
});

test("measured in one mode wins over a failure in another (Jena: CLI fail + HTTP ok)", () => {
  const comps = [
    // CLI matrix — fuseki failed to load
    comparison(
      "sp2b",
      [
        { id: "sparq", label: "sparq", mode: CLI, status: "ok" },
        { id: "fuseki", label: "Jena", mode: HTTP, status: "failed", failure: "x" },
      ],
      [{ sparq: 100, fuseki: null }],
    ),
    // HTTP twin — fuseki produced a timing
    comparison(
      "sp2b-http",
      [
        { id: "sparq", label: "sparq", mode: HTTP, status: "ok" },
        { id: "fuseki", label: "Jena", mode: HTTP, status: "ok" },
      ],
      [{ sparq: 120, fuseki: 300 }],
    ),
  ];
  const m = deriveCoverage(comps);
  const cell = m.cells["sp2b"]["fuseki"];
  assert.equal(cell.state, "measured");
  assert.equal(cell.mode, "http");
});

test("timings in both CLI and HTTP report mode 'both'", () => {
  const comps = [
    comparison(
      "watdiv",
      [{ id: "oxigraph", label: "Oxigraph", mode: CLI, status: "ok" }],
      [{ oxigraph: 200 }],
    ),
    comparison(
      "watdiv-http",
      [{ id: "oxigraph", label: "Oxigraph", mode: HTTP, status: "ok" }],
      [{ oxigraph: 250 }],
    ),
  ];
  const m = deriveCoverage(comps);
  assert.equal(m.cells["watdiv"]["oxigraph"].mode, "both");
});

test("a zero/negative timing is not counted as measured", () => {
  const comps = [
    comparison(
      "sp2b",
      [{ id: "oxigraph", label: "Oxigraph", mode: CLI, status: "ok" }],
      [{ oxigraph: 0 }],
    ),
  ];
  const m = deriveCoverage(comps);
  assert.equal(m.cells["sp2b"]["oxigraph"].state, "pending");
});
