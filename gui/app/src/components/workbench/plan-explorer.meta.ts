// [FABLE-5] sq-ixc3.19 — the Plan explorer's honesty-metadata override, split out of the
// panel file so the tool-panel registry can read it EAGERLY (the rail/tab/palette honesty
// read path needs it at first paint) while the panel component itself stays behind a lazy
// dynamic import() and out of the first-load bundle (the sq-5lyme seam).

import type { ToolOverride } from "@/data/tools";

/**
 * The Plan explorer ships working: EXPLAIN / EXPLAIN ANALYZE tree over the live in-tab
 * store (exact rows + q-error; wall times honestly unmeasured on wasm32), desktop-native
 * ANALYZE with real per-operator wall time, endpoint-mode explain against a running
 * sparq-server, the this-workbench query monitor, and an endpoint-mode all-clients
 * monitor backed by sparq-server's opt-in `/queries` registry.
 */
export const PLAN_TOOL_OVERRIDE: ToolOverride = {
  tier: "live",
  built: true,
  group: "working",
};
