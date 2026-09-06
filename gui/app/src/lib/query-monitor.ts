// [FABLE-5] sq-ixc3.19 — the live query monitor: a tiny observable registry of the
// queries THIS workbench session has in flight, each with a working Kill.
//
// HONEST SCOPE (stated in the panel UI too): this tracks queries ISSUED FROM THIS
// WORKBENCH — the in-tab wasm runs (cooperatively cancelled between streamed batches;
// the same mechanism as the Query tool's Stop) and endpoint-mode requests (killed by
// aborting the underlying `fetch`).
// [SONNET-4.6] sq-k4qo4 — endpoint mode additionally consumes sparq-server's opt-in
// server-side `/queries` registry through the small HTTP helpers at the end of this module.
//
// Plain module state + subscriber set (not React context) so it is unit-testable with
// the node runner and consumable from React via `useSyncExternalStore`.

/** Where a tracked query is executing. */
export type QueryTarget = "local" | "native" | "endpoint";

/** One in-flight query. */
export interface RunningQuery {
  /** Monotonic per-session id. */
  id: number;
  /** Short human label (the tool + mode that started it). */
  label: string;
  /** The SPARQL text (the panel truncates for display; never logged elsewhere). */
  sparql: string;
  target: QueryTarget;
  /** performance.now() at start — for a live elapsed readout (measured, not a benchmark). */
  startedAt: number;
  /** Abort handle: local runs check it between streamed batches; endpoint runs abort fetch. */
  controller: AbortController;
}

let nextId = 1;
let running: RunningQuery[] = [];
const listeners = new Set<() => void>();

function emit(): void {
  for (const l of listeners) l();
}

/** Subscribe to registry changes (returns the unsubscribe). `useSyncExternalStore`-shaped. */
export function subscribeRunning(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The current in-flight list (a stable snapshot reference until the next change). */
export function getRunning(): RunningQuery[] {
  return running;
}

/**
 * Track a query for its lifetime: registers it, and returns its abort signal + the
 * `finish` the issuer MUST call (in a `finally`) to deregister. The signal must actually
 * be wired into the run (RunOptions.signal / the fetch init) — the registry can only
 * kill what listens.
 */
export function trackQuery(
  label: string,
  sparql: string,
  target: QueryTarget,
): { id: number; signal: AbortSignal; finish: () => void } {
  const entry: RunningQuery = {
    id: nextId++,
    label,
    sparql,
    target,
    startedAt: typeof performance !== "undefined" ? performance.now() : 0,
    controller: new AbortController(),
  };
  running = [...running, entry];
  emit();
  return {
    id: entry.id,
    signal: entry.controller.signal,
    finish: () => {
      running = running.filter((r) => r.id !== entry.id);
      emit();
    },
  };
}

/**
 * Kill one tracked query: aborts its controller (the wasm run cancels at the next batch
 * boundary; an endpoint fetch rejects with AbortError). The entry stays listed until its
 * issuer's `finish` runs — the list reflects what is genuinely still executing, not an
 * optimistic removal.
 */
export function killQuery(id: number): boolean {
  const entry = running.find((r) => r.id === id);
  if (!entry) return false;
  entry.controller.abort();
  return true;
}

/** TEST-ONLY: reset module state between node-test cases. */
export function resetQueryMonitorForTests(): void {
  running = [];
  nextId = 1;
  listeners.clear();
}

/** One redacted row returned by sparq-server's opt-in running-query registry. */
export interface EndpointRunningQuery {
  id: string;
  kind: "query" | "update";
  /** Unix epoch milliseconds. */
  start: number;
  /** Stable server-provided fingerprint; raw SPARQL is deliberately never exposed. */
  fingerprint: string;
  elapsed_ms: number;
}

function registryUrl(endpointUrl: string): string {
  const endpoint = new URL(endpointUrl);
  if (endpoint.protocol !== "http:" && endpoint.protocol !== "https:") {
    throw new Error("Endpoint URL must use http:// or https://.");
  }
  const segments = endpoint.pathname.split("/").filter(Boolean);
  // [SONNET-4.6] Assume the registry is a sibling of the SPARQL endpoint.
  if (segments.length > 0) segments.pop();
  endpoint.pathname = `/${[...segments, "queries"].join("/")}`;
  endpoint.search = "";
  endpoint.hash = "";
  return endpoint.toString();
}

function registryHeaders(token?: string): Record<string, string> {
  const headers: Record<string, string> = { Accept: "application/json" };
  const trimmed = token?.trim();
  if (trimmed) headers.Authorization = `Bearer ${trimmed}`;
  return headers;
}

function parseRegistryRow(value: unknown): EndpointRunningQuery | null {
  if (typeof value !== "object" || value === null) return null;
  const row = value as Record<string, unknown>;
  if (
    typeof row.id !== "string" ||
    (row.kind !== "query" && row.kind !== "update") ||
    typeof row.start !== "number" ||
    !Number.isFinite(row.start) ||
    typeof row.fingerprint !== "string" ||
    typeof row.elapsed_ms !== "number" ||
    !Number.isFinite(row.elapsed_ms)
  ) {
    return null;
  }
  return {
    id: row.id,
    kind: row.kind,
    start: row.start,
    fingerprint: row.fingerprint,
    elapsed_ms: row.elapsed_ms,
  };
}

/** List all clients' currently running work from sparq-server's `/queries` registry. */
export async function fetchEndpointRunningQueries(
  endpointUrl: string,
  token?: string,
  fetchImpl: typeof fetch = fetch,
  signal?: AbortSignal,
): Promise<EndpointRunningQuery[]> {
  const response = await fetchImpl(registryUrl(endpointUrl), {
    method: "GET",
    headers: registryHeaders(token),
    signal,
  });
  if (!response.ok) {
    throw new QueryRegistryHttpError("Running-query list", response.status);
  }
  const body: unknown = await response.json();
  const queries =
    typeof body === "object" && body !== null
      ? (body as { queries?: unknown }).queries
      : undefined;
  if (!Array.isArray(queries)) {
    throw new Error("Running-query list returned an invalid response.");
  }
  const parsed = queries.map(parseRegistryRow);
  if (parsed.some((row) => row === null)) {
    throw new Error("Running-query list returned an invalid row.");
  }
  return parsed as EndpointRunningQuery[];
}

/** Cooperatively cancel one server-side query or update through `/queries/{id}`. */
export async function killEndpointRunningQuery(
  endpointUrl: string,
  id: string,
  token?: string,
  fetchImpl: typeof fetch = fetch,
  signal?: AbortSignal,
): Promise<void> {
  const url = `${registryUrl(endpointUrl)}/${encodeURIComponent(id)}`;
  const response = await fetchImpl(url, {
    method: "DELETE",
    headers: registryHeaders(token),
    signal,
  });
  if (!response.ok) {
    throw new QueryRegistryHttpError("Kill", response.status);
  }
}

/** An HTTP failure returned by sparq-server's optional query registry. */
export class QueryRegistryHttpError extends Error {
  constructor(
    operation: string,
    readonly status: number,
  ) {
    super(`${operation} failed (HTTP ${status}).`);
    this.name = "QueryRegistryHttpError";
  }
}
