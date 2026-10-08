// [SONNET-4.6] #3602 — SELECT result graph derivation acceptance coverage.

import assert from "node:assert/strict";
import test from "node:test";
import type { SparqlResults } from "@sparq/client";
import { deriveSelectResultGraph } from "./select-result-graph.js";

test("derives nodes and real row relationships from an entity-shaped SELECT", () => {
  const results: SparqlResults = {
    head: { vars: ["person", "knows"] },
    results: {
      bindings: [
        {
          person: { type: "uri", value: "http://example.com/alice" },
          knows: { type: "uri", value: "http://example.com/bob" },
        },
        {
          person: { type: "uri", value: "http://example.com/alice" },
          knows: { type: "uri", value: "http://example.com/bob" },
        },
      ],
    },
  };

  const graph = deriveSelectResultGraph(results);
  assert.ok(graph);
  assert.equal(graph.nodes.length, 2);
  assert.deepEqual(graph.nodes.map((node) => node.label), ["alice", "bob"]);
  assert.equal(graph.edges.length, 1);
  assert.equal(graph.edges[0].label, "knows");
  assert.equal(graph.edges[0].count, 2);
});

test("declines literal-only aggregate-shaped SELECT results", () => {
  const results: SparqlResults = {
    head: { vars: ["label", "total"] },
    results: {
      bindings: [{ label: { type: "literal", value: "people" }, total: { type: "literal", value: "4" } }],
    },
  };
  assert.equal(deriveSelectResultGraph(results), null);
});

test("keeps literals that differ only by base direction as distinct nodes", () => {
  const a = { type: "uri" as const, value: "http://example.com/a" };
  const hi = (dir?: string) => ({
    type: "literal" as const,
    value: "hi",
    "xml:lang": "en",
    "its:dir": dir,
  });
  const results: SparqlResults = {
    head: { vars: ["s", "v"] },
    results: {
      bindings: [
        { s: a, v: hi("ltr") },
        { s: a, v: hi("rtl") },
        { s: a, v: hi() },
      ],
    },
  };
  const graph = deriveSelectResultGraph(results);
  assert.ok(graph);
  assert.equal(graph.nodes.length, 4);
});
