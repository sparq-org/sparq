// [OPUS-4.8] sq-vw3ax.10 — unit tests for the PURE node-link derivation behind the /try Graph view
// (src/lib/result-graph.ts). These cover the shape the SVG renderer draws: distinct-term nodes,
// per-row adjacent-column edges, the entity-relationship gate that makes it the complement of the
// aggregate mini-viz, and the deterministic circular layout. The query SEMANTICS are proven by the
// Rust engine tests; here we only test the JS derivation over the SPARQL-1.1-JSON the binding
// returns. Run via `npm run test:unit`.
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  deriveGraph,
  circularLayout,
  MAX_GRAPH_NODES,
} from "../src/lib/result-graph.ts";
import { isGraphShaped } from "../src/lib/result-graph-shape.ts";

const XSD = "http://www.w3.org/2001/XMLSchema#";

// A resource-linking SELECT: three developers who each `:wrote` a component — the classic
// entity-relationship shape the Graph view is for.
const WROTE = {
  head: { vars: ["dev", "component"] },
  results: {
    bindings: [
      {
        dev: { type: "uri", value: "http://sparq.dev/demo/ada" },
        component: { type: "uri", value: "http://sparq.dev/demo/parser" },
      },
      {
        dev: { type: "uri", value: "http://sparq.dev/demo/ada" },
        component: { type: "uri", value: "http://sparq.dev/demo/planner" },
      },
      {
        dev: { type: "uri", value: "http://sparq.dev/demo/grace" },
        component: { type: "uri", value: "http://sparq.dev/demo/optimizer" },
      },
    ],
  },
};

test("deriveGraph: distinct terms become nodes, adjacent bound columns become edges", () => {
  const g = deriveGraph(WROTE);
  assert.ok(g, "a resource-linking result yields a graph");
  // ada, parser, planner, grace, optimizer = 5 distinct terms (ada is shared across two rows).
  assert.equal(g.nodes.length, 5);
  assert.equal(g.edges.length, 3);
  // The shared `ada` node merges (it is NOT duplicated), which is the whole point of a graph.
  const ada = g.nodes.filter((n) => n.term.value.endsWith("/ada"));
  assert.equal(ada.length, 1);
  // Every edge is labelled by the target column's variable and is a real row co-occurrence.
  for (const e of g.edges) {
    assert.equal(e.label, "component");
    assert.equal(e.count, 1);
  }
});

test("deriveGraph: nodes are CURIE-labelled but keep the raw term", () => {
  const g = deriveGraph(WROTE);
  const ada = g.nodes.find((n) => n.term.value.endsWith("/ada"));
  // The demo namespace is not one of the well-known prefixes, so it is NOT abbreviated — but a
  // foaf/rdf/xsd IRI would be. The raw value is always preserved for tooltips/export.
  assert.equal(ada.kind, "uri");
  assert.equal(ada.term.value, "http://sparq.dev/demo/ada");
});

test("deriveGraph: a repeated (source,target,label) edge counts occurrences instead of duplicating", () => {
  const dup = {
    head: { vars: ["a", "b"] },
    results: {
      bindings: [
        { a: { type: "uri", value: "http://ex/x" }, b: { type: "uri", value: "http://ex/y" } },
        { a: { type: "uri", value: "http://ex/x" }, b: { type: "uri", value: "http://ex/y" } },
      ],
    },
  };
  const g = deriveGraph(dup);
  assert.equal(g.nodes.length, 2);
  assert.equal(g.edges.length, 1);
  assert.equal(g.edges[0].count, 2);
});

test("deriveGraph: a pure label→number aggregate has no resource node, so it draws no graph", () => {
  // `?name (SUM(?loc) AS ?total)` — two literal columns. The mini-viz charts this; the graph view
  // must decline (returns null) so the same rows are never rendered two ways.
  const aggregate = {
    head: { vars: ["name", "total"] },
    results: {
      bindings: [
        {
          name: { type: "literal", value: "Ada" },
          total: { type: "literal", value: "7300", datatype: `${XSD}integer` },
        },
        {
          name: { type: "literal", value: "Grace" },
          total: { type: "literal", value: "5300", datatype: `${XSD}integer` },
        },
      ],
    },
  };
  assert.equal(deriveGraph(aggregate), null);
});

test("deriveGraph: needs ≥2 columns, ≥1 row, and ≥1 edge", () => {
  assert.equal(
    deriveGraph({ head: { vars: ["s"] }, results: { bindings: [{ s: { type: "uri", value: "http://ex/a" } }] } }),
    null,
    "a single-column result is not a graph",
  );
  assert.equal(
    deriveGraph({ head: { vars: ["s", "o"] }, results: { bindings: [] } }),
    null,
    "an empty result is not a graph",
  );
  // Two columns but every row binds only one of them → no adjacent bound pair → no edge → null.
  const noPair = {
    head: { vars: ["s", "o"] },
    results: {
      bindings: [
        { s: { type: "uri", value: "http://ex/a" } },
        { o: { type: "uri", value: "http://ex/b" } },
      ],
    },
  };
  assert.equal(deriveGraph(noPair), null);
});

test("deriveGraph: an unbound middle column does not break the chain", () => {
  // `?s ?p ?o` where ?p is unbound in the row: s and o are still adjacent BOUND columns, so the
  // edge s→o is drawn (labelled by ?o) rather than the row being dropped.
  const optionalMid = {
    head: { vars: ["s", "p", "o"] },
    results: {
      bindings: [
        {
          s: { type: "uri", value: "http://ex/s" },
          o: { type: "uri", value: "http://ex/o" },
        },
      ],
    },
  };
  const g = deriveGraph(optionalMid);
  assert.ok(g);
  assert.equal(g.edges.length, 1);
  assert.equal(g.edges[0].label, "o");
});

test("deriveGraph: a row binding two columns to the SAME term draws no self-loop", () => {
  const selfRef = {
    head: { vars: ["a", "b"] },
    results: {
      bindings: [
        { a: { type: "uri", value: "http://ex/same" }, b: { type: "uri", value: "http://ex/same" } },
      ],
    },
  };
  // The only pair is a self-relationship, so there is no edge → the result is not graph-shaped.
  assert.equal(deriveGraph(selfRef), null);
});

test("deriveGraph: literals with the same value but different datatype are distinct nodes", () => {
  const g = deriveGraph({
    head: { vars: ["s", "v"] },
    results: {
      bindings: [
        {
          s: { type: "uri", value: "http://ex/a" },
          v: { type: "literal", value: "5", datatype: `${XSD}integer` },
        },
        {
          s: { type: "uri", value: "http://ex/a" },
          v: { type: "literal", value: "5" },
        },
      ],
    },
  });
  assert.ok(g);
  // http://ex/a (shared) + "5"^^integer + "5" (plain) = 3 distinct nodes.
  assert.equal(g.nodes.length, 3);
});

test("deriveGraph: literals differing only by base direction are distinct nodes", () => {
  const a = { type: "uri", value: "http://ex/a" };
  const hi = (dir) => ({ type: "literal", value: "hi", "xml:lang": "en", "its:dir": dir });
  const g = deriveGraph({
    head: { vars: ["s", "v"] },
    results: {
      bindings: [
        { s: a, v: hi("ltr") },
        { s: a, v: hi("rtl") },
        { s: a, v: hi(undefined) },
      ],
    },
  });
  assert.ok(g);
  // http://ex/a + "hi"@en--ltr + "hi"@en--rtl + "hi"@en = 4 distinct nodes.
  assert.equal(g.nodes.length, 4);
});

test("deriveGraph: delimiter-containing lexical/datatype fields keep terms distinct (no key collision)", () => {
  // Regression: the term identity key must be INJECTIVE. It once concatenated (value, datatype,
  // lang) with a single-character delimiter, so a delimiter INSIDE one field could forge another
  // term's key and merge two genuinely distinct literals into one node. The injective structural
  // encoding must keep them apart under both attack shapes. The NUL char is built at runtime via
  // fromCharCode so this SOURCE stays plain text (no raw NUL byte). Each case is
  // [value_a, datatype_a, value_b, datatype_b], chosen so a and b collide if their fields are
  // joined by the named single-character delimiter:
  const NUL = String.fromCharCode(0);
  const cases = [
    [`a${NUL}b`, "c", "a", `b${NUL}c`], // collides under a raw-NUL join
    ["a b", "c", "a", "b c"], // collides under a space join
  ];
  for (const [av, adt, bv, bdt] of cases) {
    const g = deriveGraph({
      head: { vars: ["s", "v"] },
      results: {
        bindings: [
          { s: { type: "uri", value: "http://ex/a" }, v: { type: "literal", value: av, datatype: adt } },
          { s: { type: "uri", value: "http://ex/a" }, v: { type: "literal", value: bv, datatype: bdt } },
        ],
      },
    });
    assert.ok(g, "the resource-linking result yields a graph");
    // http://ex/a (shared) + two DISTINCT literals = 3 nodes. A key collision would merge to 2.
    assert.equal(g.nodes.length, 3, "delimiter-containing fields must not collide into one node");
    assert.equal(g.edges.length, 2, "each distinct literal keeps its own edge from the shared subject");
  }
});

test("deriveGraph: more than MAX_GRAPH_NODES distinct terms is capped and flagged truncated", () => {
  // Build a star: one hub linked to (MAX + 10) distinct leaves. Distinct terms = hub + leaves.
  const leaves = MAX_GRAPH_NODES + 10;
  const bindings = [];
  for (let i = 0; i < leaves; i++) {
    bindings.push({
      hub: { type: "uri", value: "http://ex/hub" },
      leaf: { type: "uri", value: `http://ex/leaf/${i}` },
    });
  }
  const star = { head: { vars: ["hub", "leaf"] }, results: { bindings } };
  const g = deriveGraph(star);
  assert.ok(g);
  assert.equal(isGraphShaped(star), true, "the capped predicate agrees the star is drawable");
  assert.equal(g.nodes.length, MAX_GRAPH_NODES, "node set is capped");
  assert.equal(g.truncated, true);
  assert.equal(g.totalNodes, leaves + 1, "totalNodes counts every distinct term seen, pre-cap");
  // No edge may reference a dropped node.
  const ids = new Set(g.nodes.map((n) => n.id));
  for (const e of g.edges) {
    assert.ok(ids.has(e.source) && ids.has(e.target));
  }
});

// [review #3601] Regression for the cap-boundary disagreement: a result can bind MAX_GRAPH_NODES
// distinct terms FIRST (here via single-bound rows, which produce no edges), so its ONLY qualifying
// edge sits between two cap-EXCLUDED terms that deriveGraph drops. The old uncapped predicate said
// "graph-shaped", the derivation returned null, and the hero offered a Graph toggle that rendered
// blank. The predicate now replays the cap, so BOTH must decline this result.
test("isGraphShaped/deriveGraph: an edge only between cap-excluded terms is NOT graph-shaped", () => {
  const bindings = [];
  // MAX_GRAPH_NODES distinct resources, each in a single-bound row → cap fills with edge-less nodes.
  for (let i = 0; i < MAX_GRAPH_NODES; i++) {
    bindings.push({ s: { type: "uri", value: `http://ex/filler/${i}` } });
  }
  // The only adjacent bound pair — both terms are NEW, so both fall past the cap and are dropped.
  bindings.push({
    s: { type: "uri", value: "http://ex/late/a" },
    o: { type: "uri", value: "http://ex/late/b" },
  });
  const r = { head: { vars: ["s", "o"] }, results: { bindings } };
  assert.equal(isGraphShaped(r), false, "the only edge is undrawable, so no Graph toggle");
  assert.equal(deriveGraph(r), null, "derivation agrees: nothing to draw");
});

test("isGraphShaped/deriveGraph: a post-cap edge whose endpoints WERE admitted still draws", () => {
  // Same shape, but the late edge reuses two already-admitted terms — drawable, so both say yes,
  // and the derived graph is non-empty (the toggle never offers a blank picture).
  const bindings = [];
  for (let i = 0; i < MAX_GRAPH_NODES; i++) {
    bindings.push({ s: { type: "uri", value: `http://ex/filler/${i}` } });
  }
  bindings.push({
    s: { type: "uri", value: "http://ex/filler/0" },
    o: { type: "uri", value: "http://ex/filler/1" },
  });
  const r = { head: { vars: ["s", "o"] }, results: { bindings } };
  assert.equal(isGraphShaped(r), true);
  const g = deriveGraph(r);
  assert.ok(g, "an admitted-endpoint edge keeps the result drawable");
  assert.ok(g.edges.length > 0, "whenever the toggle is offered, the graph has edges");
});

// [review #3601] The cheap eligibility predicate the hero uses to decide whether to OFFER the Graph
// toggle — WITHOUT eagerly running the full deriveGraph node/edge construction for every result.
// Its whole contract is that it must AGREE with `deriveGraph(...) !== null` (deriveGraph uses it as
// its precondition gate, and the predicate exactly replays the MAX_GRAPH_NODES admission cap), so
// the toggle never appears for a result that would draw no or an empty graph.
test("isGraphShaped: agrees with deriveGraph!==null across the derivation fixtures", () => {
  const noPair = {
    head: { vars: ["s", "o"] },
    results: {
      bindings: [
        { s: { type: "uri", value: "http://ex/a" } },
        { o: { type: "uri", value: "http://ex/b" } },
      ],
    },
  };
  const selfRef = {
    head: { vars: ["a", "b"] },
    results: {
      bindings: [
        { a: { type: "uri", value: "http://ex/same" }, b: { type: "uri", value: "http://ex/same" } },
      ],
    },
  };
  const aggregate = {
    head: { vars: ["name", "total"] },
    results: {
      bindings: [
        { name: { type: "literal", value: "Ada" }, total: { type: "literal", value: "7300", datatype: `${XSD}integer` } },
      ],
    },
  };
  const optionalMid = {
    head: { vars: ["s", "p", "o"] },
    results: {
      bindings: [{ s: { type: "uri", value: "http://ex/s" }, o: { type: "uri", value: "http://ex/o" } }],
    },
  };
  const singleCol = {
    head: { vars: ["s"] },
    results: { bindings: [{ s: { type: "uri", value: "http://ex/a" } }] },
  };
  const empty = { head: { vars: ["s", "o"] }, results: { bindings: [] } };

  // Graph-shaped ⇒ true; not graph-shaped ⇒ false — and it must MATCH deriveGraph in every case.
  for (const [label, r, expected] of [
    ["WROTE (resource→resource)", WROTE, true],
    ["optional middle column", optionalMid, true],
    ["no adjacent bound pair", noPair, false],
    ["self-reference only", selfRef, false],
    ["pure literal aggregate", aggregate, false],
    ["single column", singleCol, false],
    ["empty result", empty, false],
  ]) {
    assert.equal(isGraphShaped(r), expected, `${label}: isGraphShaped`);
    assert.equal(
      isGraphShaped(r),
      deriveGraph(r) !== null,
      `${label}: isGraphShaped must agree with deriveGraph!==null`,
    );
  }
});

test("circularLayout: deterministic, centred, inside the box", () => {
  const pts = circularLayout(4, 620, 380, 96);
  assert.equal(pts.length, 4);
  // First node sits at the top (x = centre, y < centre).
  assert.ok(Math.abs(pts[0].x - 310) < 1e-6);
  assert.ok(pts[0].y < 190);
  // Deterministic: same inputs → identical output.
  assert.deepEqual(circularLayout(4, 620, 380, 96), pts);
  // Every point lies within the box.
  for (const p of pts) {
    assert.ok(p.x >= 0 && p.x <= 620 && p.y >= 0 && p.y <= 380);
  }
});

test("circularLayout: degenerate sizes are safe (single node centred, zero nodes empty)", () => {
  assert.deepEqual(circularLayout(1, 620, 380, 96), [{ x: 310, y: 190 }]);
  assert.deepEqual(circularLayout(0, 620, 380, 96), []);
});
