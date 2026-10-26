import { test } from "node:test";
import assert from "node:assert/strict";
import { rewriteForGraphs } from "../src/lib/solid-acl.ts";

const graph = "https://alice.pod.example/health/records";

test("ASK accepts an opening group with or without WHERE", () => {
  for (const [head, rewrittenHead] of [
    ["ASK ", `ASK FROM NAMED <${graph}>\n`],
    ["ASK", `ASK\nFROM NAMED <${graph}>\n`],
    ["ask\n", `ask\nFROM NAMED <${graph}>\n`],
    ["ASK WHERE ", `ASK FROM NAMED <${graph}>\nWHERE `],
    ["ask where\n", `ask FROM NAMED <${graph}>\nwhere\n`],
  ]) {
    const group = "{ GRAPH ?g { ?s ?p ?o } }";
    assert.equal(rewriteForGraphs(head + group, [graph]), rewrittenHead + group);
  }
  assert.equal(rewriteForGraphs("ASK{}", []), "ASK\nFROM NAMED <urn:sparq:nothing>\n{}");
});

test("ASK inserts before its outer group, skipping prologue IRIs and comments", () => {
  const prefix = "BASE <https://example.org/WHERE>\nPREFIX ex: <https://example.org/#>\n# WHERE { ignored\nASK FROM NAMED <urn:WHERE> # { another comment\n";
  const group = '{ GRAPH ?g { ?s ?p "WHERE {" } FILTER EXISTS { ?s ?p ?o } }';
  assert.equal(
    rewriteForGraphs(prefix + group, [graph, "urn:another"]),
    `${prefix}FROM NAMED <${graph}>\nFROM NAMED <urn:another>\n${group}`,
  );
});

test("WHERE in projection literals and nested patterns is not the outer WHERE", () => {
  const prefix = `PREFIX ex: <https://example.org/#>\nSELECT ("WHERE {" AS ?label) (EXISTS { SELECT ?s WHERE { ?s ?p ?o } } AS ?found) `;
  const group = "WHERE { GRAPH ?g { ?s ?p ?o } }";
  assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
});

test("CONSTRUCT keeps its template ahead of the dataset clause", () => {
  const prefix = "CONSTRUCT { ?s ?p ?o } ";
  const group = "WHERE { GRAPH ?g { ?s ?p ?o } }";
  assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
});
