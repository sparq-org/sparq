// Heuristic SPARQL query-form classifier, shared by the engine dispatch (engine-context.tsx) and
// the Graph view tool's pre-run gate (graph-view-tool.tsx). Pure and DOM-free so it is
// unit-testable (query-form.test.ts) without React or WASM.

/** The SPARQL forms the WASM Store dispatches on (it has a separate verb per form). */
export type QueryForm = "select" | "ask" | "construct" | "describe" | "update";

/** Classify a query by its first significant keyword (after comments and PREFIX/BASE). */
export function classifyQuery(q: string): QueryForm {
  const body = q
    .replace(/(^|\s)#[^\n]*/g, " ")
    .replace(/\b(PREFIX\s+\S+\s+<[^>]*>|BASE\s+<[^>]*>)/gi, " ")
    .trim();
  const m = body.match(
    /\b(SELECT|ASK|CONSTRUCT|DESCRIBE|INSERT|DELETE|LOAD|CLEAR|CREATE|DROP|COPY|MOVE|ADD)\b/i,
  );
  const kw = m ? m[1].toUpperCase() : "SELECT";
  if (kw === "ASK") return "ask";
  if (kw === "CONSTRUCT") return "construct";
  if (kw === "DESCRIBE") return "describe";
  if (["INSERT", "DELETE", "LOAD", "CLEAR", "CREATE", "DROP", "COPY", "MOVE", "ADD"].includes(kw))
    return "update";
  return "select";
}

/**
 * Why the Graph view tool must NOT execute `query`, or `null` when it may run.
 *
 * The tool renders only graph results, so it runs CONSTRUCT and DESCRIBE and nothing else. The
 * check happens BEFORE execution: an UPDATE typed into the tool is refused instead of mutating
 * the live store while the result pane reports a no-op (#5783).
 */
export function graphViewRefusal(query: string): string | null {
  const form = classifyQuery(query);
  if (form === "construct" || form === "describe") return null;
  if (form === "update") {
    return "Not run: this is a SPARQL UPDATE, and the Graph view only runs CONSTRUCT or DESCRIBE queries. The store was not changed. Run updates from the query editor.";
  }
  return `Not run: this is a ${form.toUpperCase()} query, and the Graph view only runs CONSTRUCT or DESCRIBE queries.`;
}
