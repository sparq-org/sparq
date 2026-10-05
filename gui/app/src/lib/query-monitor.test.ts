// [FABLE-5] sq-ixc3.19 — unit tests for the live query monitor registry.
//
// Covers: track/finish lifecycle (register → listed; finish → delisted), kill (aborts
// the signal but leaves the entry listed until the issuer's finish — the list reflects
// what is genuinely still executing), subscriber notification, and the kill-unknown-id
// no-op. The panel wiring is covered by the Playwright spec.
//
// Run via:   npm run test:unit   (gui/app)
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";

import {
  fetchEndpointRunningQueries,
  getRunning,
  killEndpointRunningQuery,
  killQuery,
  resetQueryMonitorForTests,
  subscribeRunning,
  trackQuery,
} from "./query-monitor.js";

beforeEach(() => resetQueryMonitorForTests());

test("trackQuery – registers the entry; finish delists it", () => {
  const t = trackQuery("Plan · ANALYZE", "SELECT * WHERE { ?s ?p ?o }", "local");
  assert.equal(getRunning().length, 1);
  assert.equal(getRunning()[0].label, "Plan · ANALYZE");
  assert.equal(getRunning()[0].target, "local");
  assert.equal(t.signal.aborted, false);
  t.finish();
  assert.equal(getRunning().length, 0);
});

test("killQuery – aborts the signal but the entry stays until finish (honest liveness)", () => {
  const t = trackQuery("endpoint explain", "ASK { ?s ?p ?o }", "endpoint");
  const id = getRunning()[0].id;
  assert.equal(killQuery(id), true);
  assert.equal(t.signal.aborted, true, "kill aborts the tracked signal");
  // Still listed: the run is only finished when its issuer observes the abort.
  assert.equal(getRunning().length, 1);
  t.finish();
  assert.equal(getRunning().length, 0);
});

test("killQuery – unknown id is a false no-op, never a throw", () => {
  assert.equal(killQuery(999), false);
});

test("subscribeRunning – notified on track, finish, and unsubscribe stops it", () => {
  let events = 0;
  const unsub = subscribeRunning(() => {
    events += 1;
  });
  const t = trackQuery("q", "SELECT * WHERE { ?s ?p ?o }", "local");
  t.finish();
  assert.equal(events, 2, "track + finish each notify");
  unsub();
  trackQuery("q2", "SELECT * WHERE { ?s ?p ?o }", "local");
  assert.equal(events, 2, "unsubscribed listener is silent");
});

test("getRunning – returns a fresh snapshot reference per change (useSyncExternalStore contract)", () => {
  const before = getRunning();
  trackQuery("q", "SELECT * WHERE { ?s ?p ?o }", "local");
  assert.notEqual(getRunning(), before, "reference changes when the list changes");
  const during = getRunning();
  assert.equal(getRunning(), during, "reference is stable between changes");
});

test("fetchEndpointRunningQueries – derives the server root, authenticates, and parses rows", async () => {
  let requestUrl = "";
  let requestInit: RequestInit | undefined;
  const fetchImpl: typeof fetch = async (input, init) => {
    requestUrl = String(input);
    requestInit = init;
    return new Response(
      JSON.stringify({
        queries: [
          {
            id: "0000000000000001",
            kind: "query",
            start: 1_700_000_000_000,
            fingerprint: "0123456789abcdef",
            elapsed_ms: 250,
          },
        ],
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  };

  const rows = await fetchEndpointRunningQueries(
    "http://127.0.0.1:3030/sparql",
    " secret ",
    fetchImpl,
  );
  assert.equal(requestUrl, "http://127.0.0.1:3030/queries");
  assert.equal(requestInit?.method, "GET");
  assert.deepEqual(requestInit?.headers, {
    Accept: "application/json",
    Authorization: "Bearer secret",
  });
  assert.equal(rows[0]?.fingerprint, "0123456789abcdef");
});

test("fetchEndpointRunningQueries – preserves a reverse-proxy path prefix", async () => {
  let requestUrl = "";
  const fetchImpl: typeof fetch = async (input) => {
    requestUrl = String(input);
    return new Response(JSON.stringify({ queries: [] }), { status: 200 });
  };
  await fetchEndpointRunningQueries("https://example.test/sparq/sparql", undefined, fetchImpl);
  assert.equal(requestUrl, "https://example.test/sparq/queries");
});

test("fetchEndpointRunningQueries – rejects unsupported protocols", async () => {
  await assert.rejects(
    fetchEndpointRunningQueries("ftp://example.test/sparql"),
    /must use http:\/\/ or https:\/\//,
  );
});

test("fetchEndpointRunningQueries – exposes HTTP status and rejects malformed envelopes", async () => {
  for (const status of [404, 401]) {
    const unavailable: typeof fetch = async () => new Response(null, { status });
    await assert.rejects(
      fetchEndpointRunningQueries("https://example.test/sparql", undefined, unavailable),
      (error: unknown) =>
        error instanceof Error &&
        error.message === `Running-query list failed (HTTP ${status}).` &&
        "status" in error &&
        error.status === status,
    );
  }

  const malformed: typeof fetch = async () =>
    new Response(JSON.stringify({ queries: 3 }), { status: 200 });
  await assert.rejects(
    fetchEndpointRunningQueries("https://example.test/sparql", undefined, malformed),
    /invalid response/,
  );
});

test("fetchEndpointRunningQueries – forwards the abort signal", async () => {
  const controller = new AbortController();
  let requestInit: RequestInit | undefined;
  const fetchImpl: typeof fetch = async (_input, init) => {
    requestInit = init;
    return new Response(JSON.stringify({ queries: [] }), { status: 200 });
  };
  await fetchEndpointRunningQueries(
    "https://example.test/sparql",
    undefined,
    fetchImpl,
    controller.signal,
  );
  assert.equal(requestInit?.signal, controller.signal);
});

test("fetchEndpointRunningQueries – rejects malformed registry rows", async () => {
  const fetchImpl: typeof fetch = async () =>
    new Response(JSON.stringify({ queries: [{ id: 7 }] }), { status: 200 });
  await assert.rejects(
    fetchEndpointRunningQueries("http://127.0.0.1:3030/sparql", undefined, fetchImpl),
    /invalid row/,
  );
});

test("killEndpointRunningQuery – encodes the id and sends the bearer token", async () => {
  let requestUrl = "";
  let requestInit: RequestInit | undefined;
  const fetchImpl: typeof fetch = async (input, init) => {
    requestUrl = String(input);
    requestInit = init;
    return new Response(null, { status: 204 });
  };

  await killEndpointRunningQuery(
    "https://example.test/sparql",
    "query/id",
    "token",
    fetchImpl,
  );
  assert.equal(requestUrl, "https://example.test/queries/query%2Fid");
  assert.equal(requestInit?.method, "DELETE");
  assert.deepEqual(requestInit?.headers, {
    Accept: "application/json",
    Authorization: "Bearer token",
  });
});

test("killEndpointRunningQuery – forwards abort and exposes failure status", async () => {
  const controller = new AbortController();
  let requestInit: RequestInit | undefined;
  const fetchImpl: typeof fetch = async (_input, init) => {
    requestInit = init;
    return new Response(null, { status: 404 });
  };
  await assert.rejects(
    killEndpointRunningQuery(
      "https://example.test/sparql",
      "finished",
      undefined,
      fetchImpl,
      controller.signal,
    ),
    (error: unknown) =>
      error instanceof Error &&
      error.message === "Kill failed (HTTP 404)." &&
      "status" in error &&
      error.status === 404,
  );
  assert.equal(requestInit?.signal, controller.signal);
});
