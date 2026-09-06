// [OPUS-4.8] sq-11zy — live loader for the SEPARATE, lazy-loaded `sparq-rsp-wasm`
// ("W-rsp") bundle that backs the /surface/streaming-rsp page. This is NOT the lean
// `sparq_wasm` triplestore bundle (src/lib/sparq-wasm.ts) — it is its own wasm-pack
// `--target web` artifact (`sparq_rsp_wasm{.js,_bg.wasm}`), carrying the windowed
// RSP-QL stream processor and nothing else, so the landing page stays light and only
// THIS route pays its download. Loaded with the same runtime-`import(webpackIgnore)`
// strategy as the lean bundle (and the sibling W-reason bundle, src/lib/reason-wasm.ts):
// the glue never enters the page bundle and the wasm bytes' URL is passed explicitly
// (basePath-prefixed for GitHub Pages). Its wasm-pack output stays in the crate's own
// pkg/ (it is NOT part of the published @sparq-org/sparq package); the prebuild sync-wasm
// step copies it into public/wasm/rsp/.
//
// The browser tab drives the logical clock: `sparq-rsp` reads no wall clock and runs no
// async runtime, so a window only closes when the UI advances `ts` via a push (or a
// flush). The whole pipeline is a pure, replayable function of the pushed (triple, ts)
// sequence — see crates/sparq-rsp-wasm/README.md.

/** The R2S relation-to-stream operator names the `Rsp.select` binding accepts. */
export type R2S = "rstream" | "istream" | "dstream";

/**
 * [OPUS-4.8] sq-11zy — the wasm `Rsp` handle: one registered continuous SELECT over a
 * time window. Mirrors `crates/sparq-rsp-wasm`'s `#[wasm_bindgen] struct Rsp`. Stateful:
 * each {@link WasmRsp.push} advances the logical clock and returns the windows that just
 * CLOSED.
 */
export interface WasmRsp {
  /** Feed one timestamped Turtle triple; returns the closed-window JSON array as a string. */
  push(s: string, p: string, o: string, ts: number): string;
  /** End-of-stream: close every window up to the last `ts` seen; same JSON array shape. */
  flush(): string;
  /** Count of arrivals dropped as too late (every covering window had already closed). */
  lateDropped(): number;
  /** The registered SPARQL text (echo, for the UI). */
  sparql(): string;
}

interface RspModule {
  default: (opts?: { module_or_path: string | URL }) => Promise<unknown>;
  Rsp: {
    select(
      sparql: string,
      range: number,
      step: number,
      maxDelay: number,
      r2s: R2S,
    ): WasmRsp;
  };
}

function basePath(): string {
  // Mirror next.config.ts's basePath ('/sparq' on Pages, empty in local `next dev`).
  return process.env.NEXT_PUBLIC_BASE_PATH ?? "/sparq";
}

let modulePromise: Promise<RspModule> | null = null;

/**
 * [OPUS-4.8] sq-11zy — load + initialise the W-rsp wasm bundle once; subsequent calls
 * reuse the single instance. The fetch + compile + instantiate is the cold start;
 * {@link prewarmRsp} kicks it off on route mount so the first push pays none of it. A
 * load failure resets the cache so a later call can retry.
 */
export async function loadRsp(): Promise<RspModule["Rsp"]> {
  if (!modulePromise) {
    modulePromise = (async () => {
      const base = basePath();
      const gluePath = `${base}/wasm/rsp/sparq_rsp_wasm.js`;
      const wasmPath = `${base}/wasm/rsp/sparq_rsp_wasm_bg.wasm`;
      const mod = (await import(/* webpackIgnore: true */ gluePath)) as RspModule;
      await mod.default({ module_or_path: new URL(wasmPath, window.location.origin) });
      return mod;
    })();
    modulePromise.catch(() => {
      modulePromise = null; // allow retry on transient failure
    });
  }
  const mod = await modulePromise;
  return mod.Rsp;
}

/**
 * [OPUS-4.8] sq-11zy — eagerly pre-warm the W-rsp bundle without blocking render. Safe
 * to call on route mount and repeatedly: it shares the single {@link loadRsp} promise, so
 * the cold start happens at most once. Resolves when the bundle is ready (or rejects on a
 * load failure, which resets the cache so a later register can retry).
 */
export function prewarmRsp(): Promise<unknown> {
  return loadRsp();
}

/**
 * [OPUS-4.8] sq-11zy — register a continuous SELECT over a time window, in your tab via
 * the W-rsp bundle. The returned {@link WasmRsp} is stateful — each `push` advances the
 * logical clock — so callers own its lifetime (re-register to reset the stream). Throws
 * if the bundle is missing the `Rsp` export (wrong artifact) or `Rsp.select` rejects the
 * query / window parameters.
 */
export async function registerRsp(
  sparql: string,
  range: number,
  step: number,
  maxDelay: number,
  r2s: R2S,
): Promise<WasmRsp> {
  const Rsp = await loadRsp();
  if (typeof Rsp?.select !== "function") {
    throw new Error(
      "This wasm bundle does not export the streaming `Rsp` handle. " +
        "Build crates/sparq-rsp-wasm with `wasm-pack build --target web`.",
    );
  }
  return Rsp.select(sparql, range, step, maxDelay, r2s);
}
