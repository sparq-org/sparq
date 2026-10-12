"use client";

// [OPUS-4.8] sq-vw3ax.3 / sq-vw3ax — one capability affordance in /capabilities.
//
// Three shapes (data/capabilities.ts `classify`):
//   * deep  → a plain link "Open →" to the surface's retained /surface/<slug> deep page.
//   * demo  → a "Demo ▸" disclosure. The TILE is just the controlled button (DemoTileButton);
//             its interactive body (LazyDemo + a "Details & caveats" <details>) is rendered
//             SEPARATELY at full lane width by CapabilityDemoBody, so the lane grid can lay the
//             body out as a full-content-width row instead of squeezing it into a half-column
//             tile (sq-67qji — issue #1675). The lane grid (capability-lane-grid.tsx) owns the
//             open/mounted state and pairs each button with its body by slug.
//   * native → a static built-crate row with a one-line API snippet + crate/SKILL deep links.
//   * soon  → a non-interactive "Coming soon" row that links out to GitHub.
//
// [OPUS-4.8] sq-vw3ax — BOLD redesign: each affordance renders as a depth TILE (the approved
// web-capabilities.html lane layout) instead of a flat 64px row, with a per-theme accent line
// that reveals on hover (and pins on while the demo is open). The lazy-mount machinery is
// UNCHANGED — it is the load-bearing #1 risk (research/website-redesign.md §3 COLLAPSE):
// `mounted` flips true on first expand and the demo chunk is fetched only then (see
// lazy-demo.tsx). Collapsing keeps `mounted` true so a re-expand never re-fetches; the body is
// hidden with CSS rather than unmounted. The heavy bb.js / noir_js graph stays isolated to the
// ZK chunk alone.

import * as React from "react";
import Link from "next/link";
import { ArrowRight, ChevronRight, ExternalLink } from "lucide-react";

import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { TIER_LABEL, TIER_VARIANT, type Surface } from "@/data/surfaces";
import { surfaceBySlug, type CapabilityKind } from "@/data/capabilities";
import {
  LazyDemo,
  hasLazyDemo,
  type LazyDemoSlug,
} from "@/components/capabilities/lazy-demo";

const REPO_URL = "https://github.com/sparq-org/sparq";

interface RowMeta {
  /** ONE caveat sentence (honesty preserved; the full hedge lives in the SKILL/README). */
  caveat: string;
  /** Crate / source link (GitHub). */
  readme: string;
  /** SKILL.md link (GitHub). */
  skill?: string;
}

/** Per-demo one-line caveat + depth links — the honesty kept terse (research §3 COLLAPSE). */
const META: Record<LazyDemoSlug, RowMeta> = {
  geosparql: {
    caveat:
      "Captured-output walkthrough: sparq-geo is an opt-in native crate, so the static site replays real, verbatim engine output rather than running it live; planar DE-9IM, locally-exact metric distance.",
    readme: `${REPO_URL}/tree/main/crates/sparq-geo`,
    skill: `${REPO_URL}/blob/main/skills/geosparql/SKILL.md`,
  },
  "full-text": {
    caveat:
      "Runs live via the optional W-text wasm bundle, lazy-loaded on expand; BM25 over the sample graph, not a tuned production index.",
    readme: `${REPO_URL}/tree/main/crates/sparq-text`,
    skill: `${REPO_URL}/blob/main/skills/full-text/SKILL.md`,
  },
  vector: {
    caveat:
      "Captured-output walkthrough: sparq-vectors is an opt-in native crate, so the real label-embedding run and vec: result tables are replayed verbatim, not executed in the tab.",
    readme: `${REPO_URL}/tree/main/crates/sparq-vectors`,
    skill: `${REPO_URL}/blob/main/skills/vector/SKILL.md`,
  },
  genai: {
    caveat:
      "Captured-output walkthrough: the schema card and executed result tables are real engine output; the model step is a scripted fixture (sparq-nlq is an opt-in native crate).",
    readme: `${REPO_URL}/tree/main/crates/sparq-nlq`,
    skill: `${REPO_URL}/blob/main/skills/genai/SKILL.md`,
  },
  "http-server": {
    caveat:
      "Captured curl + SSE-frame walkthrough: the static site has no backend, so the endpoint's real I/O (incl. a subscription firing on a committed UPDATE) is replayed, not served live.",
    readme: `${REPO_URL}/tree/main/crates/sparq-server`,
    skill: `${REPO_URL}/blob/main/skills/http-server/SKILL.md`,
  },
  "streaming-rsp": {
    caveat:
      "Runs live via the optional RSP wasm bundle, lazy-loaded on expand; RSP-QL windows over a scripted sample stream.",
    readme: `${REPO_URL}/tree/main/crates/sparq-rsp`,
    skill: `${REPO_URL}/blob/main/skills/streaming-rsp/SKILL.md`,
  },
  federation: {
    caveat:
      "Captured-output walkthrough: the federation crates are opt-in native code (never in the wasm bundles), so the selection and plans are the real sparq-fedplan planner's verbatim output over the declared fixture — deterministic estimates, not a live multi-endpoint run and not benchmark numbers.",
    readme: `${REPO_URL}/tree/main/crates/sparq-fedplan`,
    skill: `${REPO_URL}/blob/main/skills/federated-planning/SKILL.md`,
  },
  zk: {
    caveat:
      "Research-grade: the v1 ZK verifier is NOT externally audited — sound as landed under its stated threat model, pending re-review. Indicative engineering, not an audited cryptographic guarantee. Proving runs in-tab via 3rd-party bb.js UltraHonk (the ~MB prover chunk loads only on expand).",
    readme: `${REPO_URL}/tree/main/crates/sparq-zk`,
    skill: `${REPO_URL}/blob/main/skills/zk/SKILL.md`,
  },
  mpc: {
    caveat:
      "Research-grade in-tab SIMULATION of the protocol the native sparq-mpc crate runs — a faithful illustration, not the hardened crate in your browser and not a proof of correctness (that layer is a stub). Not externally audited.",
    readme: `${REPO_URL}/tree/main/crates/sparq-mpc`,
    skill: `${REPO_URL}/blob/main/skills/mpc/SKILL.md`,
  },
  policy: {
    caveat:
      "Captured-output walkthrough: sparq-policy is an opt-in native crate (evaluate() is pure Rust, no I/O) the wasm bundles never carry, so each Permit/Deny decision is the real evaluate() output for that (policy, request) pair — pinned by a named crate test — replayed rather than run in your tab. Single-node ODRL; the federated ODRL→MPC composition is deferred.",
    readme: `${REPO_URL}/tree/main/crates/sparq-policy`,
    skill: `${REPO_URL}/blob/main/skills/usage-control-policy/SKILL.md`,
  },
};

function TierBadge({ surface }: { surface: Surface }) {
  return (
    <Badge
      variant={TIER_VARIANT[surface.tier]}
      className="h-5 shrink-0 px-2 text-[11px]"
      title={TIER_LABEL[surface.tier]}
    >
      {TIER_LABEL[surface.tier]}
    </Badge>
  );
}

/** The shared tile body: accent line + icon + title/blurb + tier badge + a trailing CTA.
 *  `interactive` toggles the hover lift (a button/link wrapper supplies the actual semantics). */
function TileShell({
  surface,
  accent,
  cta,
  details,
  interactive = true,
  active = false,
}: {
  surface: Surface;
  accent: string;
  cta: React.ReactNode;
  /** Optional compact proof/deep-link block for a built native-only capability. */
  details?: React.ReactNode;
  interactive?: boolean;
  /** Pins the accent + elevation on (used while a demo tile's body is open). */
  active?: boolean;
}) {
  const Icon = surface.icon;
  return (
    <div
      data-capability={surface.slug}
      className={cn(
        "relative flex items-start gap-3 overflow-hidden rounded-[var(--radius-lg)] border bg-card px-4 py-3.5 transition-all duration-150",
        interactive &&
          "hover:-translate-y-0.5 hover:border-primary/45 hover:shadow-elevation-2 [&:hover>[data-accent]]:opacity-90",
        active && "border-primary/45 shadow-elevation-2",
      )}
    >
      {/* The per-theme accent spine — revealed on hover, and pinned on while the demo is open. */}
      <span
        data-accent
        aria-hidden
        className={cn(
          "absolute inset-y-0 left-0 w-[3px] transition-opacity duration-150",
          active ? "opacity-90" : "opacity-0",
        )}
        style={{ background: accent }}
      />
      <span className="flex size-9 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-primary/10 text-primary">
        <Icon className="size-[18px]" aria-hidden />
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-semibold tracking-tight">{surface.title}</div>
        <div className="mt-1 text-[12.7px] leading-snug text-muted-foreground">
          {surface.blurb}
        </div>
        {details}
      </div>
      <div className="flex shrink-0 flex-col items-end gap-2">
        <TierBadge surface={surface} />
        {cta}
      </div>
    </div>
  );
}

/** [GPT-5.6] sq-vw3ax.15 — built native crate: copyable API proof + direct depth links,
 *  without inventing a demo, route, or browser bundle. */
function NativeTile({ surface, accent }: { surface: Surface; accent: string }) {
  const native = surface.native;
  if (!native) return null;

  return (
    <TileShell
      surface={surface}
      accent={accent}
      interactive={false}
      cta={null}
      details={
        <div className="mt-3 space-y-2 border-t pt-2.5">
          <code tabIndex={0} className="block overflow-x-auto rounded-md bg-muted px-2.5 py-2 font-mono text-[11.5px] text-foreground">
            {native.snippet}
          </code>
          <div className="flex flex-wrap gap-x-3 gap-y-1 text-[12px]">
            <a
              className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              href={native.readme}
              target="_blank"
              rel="noopener noreferrer"
            >
              Crate / source <ExternalLink className="size-3" aria-hidden />
            </a>
            <a
              className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              href={native.skill}
              target="_blank"
              rel="noopener noreferrer"
            >
              SKILL.md <ExternalLink className="size-3" aria-hidden />
            </a>
          </div>
        </div>
      }
    />
  );
}

const CTA_CLASS =
  "inline-flex items-center gap-1 text-[12px] font-semibold text-primary";

/** "Open →" deep-page tile — a plain link to the surface's retained deep page. */
function DeepTile({ surface, accent }: { surface: Surface; accent: string }) {
  return (
    <Link
      href={surface.href}
      className="group block rounded-[var(--radius-lg)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
    >
      <TileShell
        surface={surface}
        accent={accent}
        cta={
          <span className={CTA_CLASS}>
            Open
            <ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5" />
          </span>
        }
      />
    </Link>
  );
}

/** "Coming soon" tile — no demo yet; links out to source. */
function SoonTile({ surface, accent }: { surface: Surface; accent: string }) {
  return (
    <a
      href={REPO_URL}
      target="_blank"
      rel="noopener noreferrer"
      className="group block rounded-[var(--radius-lg)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
    >
      <TileShell
        surface={surface}
        accent={accent}
        cta={
          <span className="inline-flex items-center gap-1 text-[12px] font-medium text-muted-foreground">
            Source
            <ExternalLink className="size-3" />
          </span>
        }
      />
    </a>
  );
}

/** Internal /showcase route for the flagship demos that have a full end-to-end showcase page.
 *  Surfaced as an "Open the full showcase" link in the expanded demo, so the maintainer's
 *  "send me to the showcase" intent (issue #1675) is honoured without leaving the gallery or
 *  fabricating a showcase page for the six demos that have none. */
const SHOWCASE_HREF: Partial<Record<LazyDemoSlug, string>> = {
  zk: "/showcase/zk-car-hire",
  mpc: "/showcase/mpc-100k",
};

/** "Demo ▸" tile HEADER — a controlled disclosure button (grid cell). The interactive body is
 *  rendered separately at full lane width by `CapabilityDemoBody`, so a wide demo is no longer
 *  confined to a half-column tile (sq-67qji). Open state is owned by the lane grid, which pairs
 *  this button with its body by slug. */
function DemoTileButton({
  surface,
  accent,
  open,
  onToggle,
}: {
  surface: Surface;
  accent: string;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      className="group block w-full rounded-[var(--radius-lg)] text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
    >
      <TileShell
        surface={surface}
        accent={accent}
        active={open}
        cta={
          <span className={CTA_CLASS}>
            Demo
            <ChevronRight
              className={cn("size-3 transition-transform", open && "rotate-90")}
              aria-hidden
            />
          </span>
        }
      />
    </button>
  );
}

/** The full-lane-width body for an expanded demo (sq-67qji). The lane grid renders this as a
 *  `lg:col-span-2` row BELOW the tile grid (capability-lane-grid.tsx), reclaiming the numbered-
 *  spine column so the interactive demo gets the full content width instead of ~1/3 of it —
 *  the fix for issue #1675.
 *
 *  The lazy mount is UNCHANGED and load-bearing (the #1 risk, research/website-redesign.md §3):
 *  this body is only rendered once its slug has been opened (the lane grid only mounts opened
 *  slugs), and it stays mounted — hidden with CSS on collapse — so re-expanding never re-fetches
 *  the demo's code-split chunk. `data-demo-body` + the CSS-hidden collapse are the invariants the
 *  e2e lazy-mount test (e2e/capabilities-lazy.spec.ts) asserts, and are preserved verbatim. */
export function CapabilityDemoBody({
  slug,
  open,
  onCollapse,
}: {
  slug: LazyDemoSlug;
  open: boolean;
  onCollapse: () => void;
}) {
  const surface = surfaceBySlug(slug);
  const meta = META[slug];
  const showcaseHref = SHOWCASE_HREF[slug];
  const Icon = surface?.icon;

  return (
    <div data-demo-body={slug} className={cn("lg:col-span-2", !open && "hidden")}>
      <div className="rounded-[var(--radius-lg)] border bg-muted/20 p-4 sm:p-5">
        {/* Body header — re-anchors the now full-width demo to its tile and carries the
            flagship's "full showcase" link + a nearby collapse control (the toggle button now
            lives up in the grid, so a local collapse keeps the demo usable). */}
        <div className="mb-4 flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b pb-3">
          <div className="flex items-center gap-2 text-sm font-semibold tracking-tight">
            {Icon && (
              <span className="flex size-7 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-primary/10 text-primary">
                <Icon className="size-4" aria-hidden />
              </span>
            )}
            {surface?.title}
          </div>
          <div className="flex items-center gap-4">
            {showcaseHref && (
              <Link href={showcaseHref} className={CTA_CLASS}>
                Open the full showcase
                <ArrowRight className="size-3" aria-hidden />
              </Link>
            )}
            <button
              type="button"
              onClick={onCollapse}
              className="inline-flex items-center gap-1 text-[12px] font-medium text-muted-foreground transition-colors hover:text-foreground"
            >
              Collapse
              <ChevronRight className="size-3 -rotate-90" aria-hidden />
            </button>
          </div>
        </div>

        <LazyDemo slug={slug} mounted />
      </div>

      <details className="mt-3 rounded-[var(--radius-lg)] border bg-card px-3 py-2 text-sm">
        <summary className="cursor-pointer select-none font-medium text-foreground/90">
          Details &amp; caveats
        </summary>
        <p className="mt-2 text-muted-foreground">{meta.caveat}</p>
        <div className="mt-2 flex flex-wrap gap-3 text-sm">
          <a
            className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
            href={meta.readme}
            target="_blank"
            rel="noopener noreferrer"
          >
            Crate / source <ExternalLink className="size-3.5" />
          </a>
          {meta.skill && (
            <a
              className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              href={meta.skill}
              target="_blank"
              rel="noopener noreferrer"
            >
              SKILL.md <ExternalLink className="size-3.5" />
            </a>
          )}
        </div>
      </details>
    </div>
  );
}

// [OPUS-4.8] sq-vw3ax.3 — the boundary is `slug` (a string) + `kind` + `accent`, NOT the Surface
// object: a Surface carries a lucide `icon` function, which cannot cross the server→client RSC
// boundary (it is not serializable). The server /capabilities page passes the serializable slug
// + the theme accent; this client component resolves the full surface (icon included) by slug.
export function CapabilityRowItem({
  slug,
  kind,
  accent = "var(--primary)",
  open = false,
  onToggle,
}: {
  slug: string;
  kind: CapabilityKind;
  accent?: string;
  /** Controlled disclosure state for a demo tile — owned by the lane grid (sq-67qji). */
  open?: boolean;
  onToggle?: () => void;
}) {
  const surface = surfaceBySlug(slug);
  if (!surface) return null; // data drift — a row for an unknown surface; render nothing.
  if (kind === "deep") return <DeepTile surface={surface} accent={accent} />;
  if (kind === "native") return <NativeTile surface={surface} accent={accent} />;
  if (kind === "soon") return <SoonTile surface={surface} accent={accent} />;
  // demo — guard against a data drift where a "demo" surface lacks a registered demo.
  if (hasLazyDemo(surface.slug)) {
    return (
      <DemoTileButton
        surface={surface}
        accent={accent}
        open={open}
        onToggle={onToggle ?? (() => {})}
      />
    );
  }
  return <SoonTile surface={surface} accent={accent} />;
}
