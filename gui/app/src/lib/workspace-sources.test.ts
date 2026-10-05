// [OPUS-5] sq-ixc3.13 — unit tests for the Imports list's source-identity rule
// (workspace-sources.ts).
//
// THE PROPERTY UNDER TEST: the rail's RE-FETCH of a recorded `url` source must UPDATE that
// source, not append a second one. Before this rule `recordImport` did `[...sources, source]`
// unconditionally, so every refresh of the same URL grew another identical rail row (and the
// row key `${kind}-${label}-${importedAt}` made each one distinct, so they all rendered).
//
// Run via:   npm run test:unit   (gui/app)
import { test } from "node:test";
import assert from "node:assert/strict";

import { upsertSource } from "./workspace-sources.js";
import type { WorkspaceSourceMeta } from "@sparq/client";

function url(u: string, at: number, bytes = 10): WorkspaceSourceMeta {
  return { kind: "url", label: u, url: u, format: "turtle", bytes, importedAt: at };
}

function local(label: string, at: number): WorkspaceSourceMeta {
  return { kind: "local", label, format: "turtle", bytes: 1, importedAt: at };
}

test("upsertSource: a re-fetched URL replaces its entry in place, never appending", () => {
  const before = [url("https://a.example/d.ttl", 1), url("https://b.example/d.ttl", 2)];
  const after = upsertSource(before, {
    ...url("https://a.example/d.ttl", 99, 4096),
    format: "nquads",
  });

  assert.equal(after.length, 2, "the same URL must not add a second source");
  // Position preserved, so the rail does not reorder under a refresh.
  assert.equal(after[0].url, "https://a.example/d.ttl");
  assert.equal(after[1].url, "https://b.example/d.ttl");
  // The fresher metadata won.
  assert.equal(after[0].importedAt, 99);
  assert.equal(after[0].bytes, 4096);
  assert.equal(after[0].format, "nquads");
});

test("upsertSource: a different URL appends", () => {
  const before = [url("https://a.example/d.ttl", 1)];
  const after = upsertSource(before, url("https://b.example/d.ttl", 2));
  assert.deepEqual(
    after.map((s) => s.url),
    ["https://a.example/d.ttl", "https://b.example/d.ttl"],
  );
});

test("upsertSource: local imports always append — they have no re-fetch identity", () => {
  // Same filename twice may be two entirely different files; the browser cannot tell.
  const before = [local("data.ttl", 1)];
  const after = upsertSource(before, local("data.ttl", 2));
  assert.equal(after.length, 2);
  assert.deepEqual(
    after.map((s) => s.importedAt),
    [1, 2],
  );
});

test("upsertSource: a url source with no recorded URL appends (nothing to key on)", () => {
  const orphan: WorkspaceSourceMeta = {
    kind: "url",
    label: "unknown",
    format: "turtle",
    importedAt: 1,
  };
  const after = upsertSource([orphan], { ...orphan, importedAt: 2 });
  assert.equal(after.length, 2);
});

test("upsertSource: never mutates the input array", () => {
  const before = [url("https://a.example/d.ttl", 1)];
  const snapshot = [...before];
  upsertSource(before, url("https://a.example/d.ttl", 2));
  upsertSource(before, url("https://c.example/d.ttl", 3));
  assert.deepEqual(before, snapshot);
});
