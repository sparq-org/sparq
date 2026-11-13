// [OPUS-4.8] sq-vw3ax.6 — the /examples gallery model: the curated "show me it working"
// set, derived from the single FLAGSHIPS source (data/surfaces.ts) plus the live workbench.
//
// /examples is the curated "show me it working" gallery (research/website-redesign.md §4):
// a 2-col grid of LARGE demo cards = the 3 flagships (zk-car-hire, mpc-100k, solid-pairs) +
// a "Live SPARQL workbench" card. Each card carries a one-line OUTCOME (what you walk away
// having proven), a tier badge, and an [Open demo] CTA. The /showcase/<slug> pages are the
// detail targets (rebuilt demo-first); the workbench card targets /app (sq-4hiqe: the /try
// REPL was removed — /app is the single workbench).
//
// Keeping the three flagship items derived from FLAGSHIPS (rather than re-listed) means a
// re-word of a flagship's title/blurb/tier in surfaces.ts flows straight into this gallery, the
// Home "See it work" row, and Cmd-K together — no second source to drift.

import { PlayCircle, FileLock2, type LucideIcon } from "lucide-react";

import { FLAGSHIPS, type Tier } from "@/data/surfaces";

export interface ExampleCard {
  /** Stable key + the demo route the card opens. */
  href: string;
  title: string;
  /** One-line OUTCOME — what you walk away having seen proven (not a feature list). */
  outcome: string;
  tier: Tier;
  icon: LucideIcon;
  /** Flagships carry the ★ marker; the workbench card does not. */
  flagship: boolean;
  /**
   * [OPUS-4.8] sq-4hiqe — the card's destination is served by a SEPARATE deployed app at the
   * same origin (the /app workbench, overlaid at /app/). Such a card must be a HARD full-page
   * anchor, not a next/link soft nav (which would fetch the foreign /app/index.txt RSC payload).
   */
  external?: boolean;
}

// A per-flagship one-line OUTCOME (the "you walk away having proven X" line), keyed by slug.
// The flagship `blurb` in surfaces.ts is the longer descriptive sentence used elsewhere; the
// gallery wants the terse outcome, so it lives here, next to the gallery that consumes it.
const FLAGSHIP_OUTCOME: Record<string, string> = {
  "zk-car-hire": "Prove you may hire a car with a live zero-knowledge proof (research demo).",
  "mpc-100k":
    "Learn only whether four incomes clear £100k — no salary, not even the total, revealed.",
  "solid-pairs":
    "One query, one Pod — a different result set per (user, app) pair, enforced live.",
};

const FLAGSHIP_CARDS: ExampleCard[] = FLAGSHIPS.map((f) => ({
  href: f.href,
  title: f.title,
  outcome: FLAGSHIP_OUTCOME[f.slug] ?? f.blurb,
  tier: f.tier,
  icon: f.icon,
  flagship: true,
}));

// The workbench card — the single best proof artifact (the killer demo), opened as a large card
// alongside the flagships. [OPUS-4.8] sq-4hiqe — retargeted from the removed /try REPL to the
// /app workbench (external: a separate overlaid app, so a hard full-page anchor).
const REPL_CARD: ExampleCard = {
  href: "/app",
  title: "Live SPARQL workbench",
  outcome:
    "Run real SPARQL against a sample graph — the Rust engine compiled to wasm, in your tab.",
  tier: "live",
  icon: PlayCircle,
  flagship: false,
  external: true,
};

// [OPUS-4.8] sq-3p0z (#822) — the Verifiable-Credentials import-and-query demo. A `live`
// data/query capability (not a privacy flagship — it does no crypto), so it sits beside the
// REPL rather than in FLAGSHIPS: drag-drop / paste / fetch a VC and run SPARQL over its claims,
// in-tab. The import-vs-verify honesty lives on the page itself (and in the page's caveat).
const VC_CARD: ExampleCard = {
  href: "/showcase/verifiable-credentials",
  title: "Import & query Verifiable Credentials",
  outcome:
    "Drag in a W3C VC (JSON-LD / Turtle) and run real SPARQL over its claims — imported & queried in your tab (not verified).",
  tier: "live",
  icon: FileLock2,
  flagship: false,
};

/** The curated example gallery: the 3 flagships + the live REPL + the VC import demo. */
export const EXAMPLE_CARDS: ExampleCard[] = [...FLAGSHIP_CARDS, REPL_CARD, VC_CARD];
