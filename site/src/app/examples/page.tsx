import type { Metadata } from "next";
import * as React from "react";
import Link from "next/link";
import { ArrowRight } from "lucide-react";

// [OPUS-4.8] sq-vw3ax.6 — /examples: the curated "show me it working" gallery.
//
// A one-line lede + a 2-col grid of LARGE demo cards = the 3 flagships (zk-car-hire,
// mpc-100k, solid-pairs) + a "Live SPARQL workbench" card (research/website-redesign.md §4). Each
// card: title, one-line OUTCOME, tier badge, a decorative still, [Open demo]. No prose walls — the
// /showcase/* pages (rebuilt demo-first) and /app (the workbench, sq-4hiqe) are the detail targets
// the cards open. A card whose destination is the separate overlaid /app is a hard anchor.
//
// Design judgment (no thumbnail image assets exist in site/public): the "still/thumbnail" is a
// dependency-free, theme-token CSS panel (the surface icon over a soft gradient), not a raster
// screenshot. This keeps the static export light and avoids stale/heavy PNGs that would drift
// from the live demos; a real captured still can drop in later behind the same slot. Flagged in
// the PR body + a bead note for the maintainer to steer.
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { EXAMPLE_CARDS, type ExampleCard } from "@/data/examples";
import { withBasePath } from "@/lib/base-path";
import { TIER_LABEL, TIER_VARIANT } from "@/data/surfaces";

export const metadata: Metadata = {
  title: "Examples",
  description:
    "The curated 'show me it working' gallery — three end-to-end flagship demos (ZK car-hire, MPC £100k threshold, Solid per-pair result sets) plus the live SPARQL REPL, each runnable in your browser tab.",
};

export default function ExamplesPage() {
  return (
    <div className="space-y-8">
      <header className="space-y-3">
        <h1 className="text-3xl font-semibold tracking-tight">Examples</h1>
        <p className="measure text-muted-foreground">
          See sparq working — four runnable demos. Three end-to-end privacy flagships
          and the live SPARQL REPL, each opening into a hands-on page.
        </p>
      </header>

      <div className="grid gap-5 md:grid-cols-2">
        {EXAMPLE_CARDS.map((card) => (
          <ExampleCardItem key={card.href} card={card} />
        ))}
      </div>
    </div>
  );
}

// External cards target the separate /app build, so they need a full-page anchor:
// Next soft navigation would fetch /app/index.txt as a site RSC payload.
function ExampleCardLink({ card, children }: { card: ExampleCard; children: React.ReactNode }) {
  const className = "group block focus-visible:outline-none";
  const label = `Open demo: ${card.title}`;
  return card.external ? (
    <a
      href={withBasePath(card.href.endsWith("/") ? card.href : `${card.href}/`)}
      className={className}
      aria-label={label}
    >
      {children}
    </a>
  ) : (
    <Link href={card.href} className={className} aria-label={label}>
      {children}
    </Link>
  );
}

/** A large demo card: a decorative still over title · outcome · tier badge · [Open demo]. */
function ExampleCardItem({ card }: { card: ExampleCard }) {
  const Icon = card.icon;
  return (
    <ExampleCardLink card={card}>
      <Card className="h-full overflow-hidden pt-0 transition-colors group-hover:ring-primary/40 group-focus-visible:ring-3 group-focus-visible:ring-ring/50">
        {/* The "still" — a dependency-free token gradient with the surface icon. */}
        <div
          className="relative flex h-32 items-center justify-center bg-gradient-to-br from-primary/15 via-primary/5 to-transparent"
          aria-hidden
        >
          <span className="flex size-14 items-center justify-center rounded-2xl bg-background/70 text-primary shadow-sm ring-1 ring-foreground/10 backdrop-blur">
            <Icon className="size-7" />
          </span>
          <span className="absolute right-3 top-3">
            <Badge variant={TIER_VARIANT[card.tier]}>{TIER_LABEL[card.tier]}</Badge>
          </span>
        </div>
        <CardHeader className="gap-2">
          <CardTitle className="flex items-center gap-1.5 text-lg">
            {card.flagship && (
              <span className="text-[var(--warning)]" aria-hidden>
                ★
              </span>
            )}
            {card.title}
          </CardTitle>
          <CardDescription className="leading-relaxed">{card.outcome}</CardDescription>
        </CardHeader>
        <CardContent>
          <span className="inline-flex items-center gap-1 text-sm font-medium text-primary">
            Open demo
            <ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
          </span>
        </CardContent>
      </Card>
    </ExampleCardLink>
  );
}
