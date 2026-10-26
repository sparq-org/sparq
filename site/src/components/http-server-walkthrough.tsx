"use client";

// [OPUS-4.8] sq-rnwc — the /surface/http-server walkthrough (tier-e, captured I/O).
// There is no hosted sparq-server behind the static Pages site, so — per the
// feature-showcase design's honest tier-e fallback — this replays REAL captured I/O
// rather than mocking a live endpoint:
//
//   1. REST RECIPES — the SPARQL 1.1 Protocol + Graph Store + EXPLAIN + /metrics curl
//      commands, each with its verbatim recorded response.
//   2. LIVE-SUBSCRIPTION REPLAY — step through the captured SSE transcript: open the
//      stream, see the sequence-0 snapshot, "commit" an UPDATE, watch the sequence-1
//      incremental diff arrive. The "Play" button advances the frames on a timer so the
//      subscription visibly "fires"; nothing leaves the tab.
//
// All payloads come from src/lib/http-server.ts (framework-free, unit-tested).

import * as React from "react";
import { Play, RotateCcw, Terminal, Radio, ArrowRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import {
  RECIPES,
  SUBSCRIPTION_TRANSCRIPT,
  type Recipe,
  type SubFrame,
} from "@/lib/http-server";

function langBadge(lang: Recipe["lang"]): { label: string; variant: "muted" | "outline" } {
  const label =
    lang === "json"
      ? "SPARQL-JSON"
      : lang === "turtle"
        ? "Turtle"
        : lang === "csv"
          ? "CSV"
          : lang === "http"
            ? "HTTP"
            : "text/plain";
  return { label, variant: "outline" };
}

function RecipeCard({ recipe }: { recipe: Recipe }) {
  const badge = langBadge(recipe.lang);
  return (
    <Card>
      <CardHeader className="space-y-1">
        <div className="flex items-center justify-between gap-2">
          <CardTitle className="text-base">{recipe.title}</CardTitle>
          <Badge variant={badge.variant} className="font-mono text-[11px]">
            {badge.label}
          </Badge>
        </div>
        <p className="text-sm text-muted-foreground">{recipe.blurb}</p>
      </CardHeader>
      <CardContent className="space-y-2">
        <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Terminal className="size-3.5" aria-hidden="true" />
          request
        </div>
        <pre className="overflow-x-auto rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed">
          {recipe.curl}
        </pre>
        <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <ArrowRight className="size-3.5" aria-hidden="true" />
          response
        </div>
        <pre className="overflow-x-auto rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed">
          {recipe.response}
        </pre>
      </CardContent>
    </Card>
  );
}

const FRAME_SIDE: Record<
  SubFrame["side"],
  { label: string; dot: string; variant: "success" | "default" | "muted" }
> = {
  client: { label: "client", dot: "bg-primary", variant: "default" },
  server: { label: "server", dot: "bg-[var(--success)]", variant: "success" },
  note: { label: "action", dot: "bg-[var(--warning)]", variant: "muted" },
};

function TranscriptFrame({
  frame,
  active,
}: {
  frame: SubFrame;
  active: boolean;
}) {
  const meta = FRAME_SIDE[frame.side];
  return (
    <div
      className={cn(
        "rounded-lg border p-3 transition-colors",
        frame.side === "note" ? "bg-[var(--warning)]/5" : "bg-muted/40",
        active && "ring-2 ring-ring/50",
      )}
    >
      <div className="mb-1.5 flex items-center gap-2">
        <span className={cn("size-2 rounded-full", meta.dot)} aria-hidden="true" />
        <Badge variant={meta.variant} className="text-[10px] uppercase">
          {meta.label}
        </Badge>
        <span className="font-mono text-[12.5px] font-medium">{frame.label}</span>
      </div>
      <pre className="overflow-x-auto font-mono text-[12px] leading-relaxed text-muted-foreground">
        {frame.body}
      </pre>
    </div>
  );
}

export function HttpServerWalkthrough() {
  // step = number of transcript frames revealed so far (0 … length).
  const [step, setStep] = React.useState(0);
  const [playing, setPlaying] = React.useState(false);

  React.useEffect(() => {
    if (!playing) return;
    const t = setTimeout(() => {
      setStep(step + 1);
      if (step + 1 >= SUBSCRIPTION_TRANSCRIPT.length) setPlaying(false);
    }, 850);
    return () => clearTimeout(t);
  }, [playing, step]);

  const done = step >= SUBSCRIPTION_TRANSCRIPT.length;

  function play() {
    if (done) setStep(0);
    setPlaying(true);
  }
  function reset() {
    setPlaying(false);
    setStep(0);
  }

  return (
    <div className="space-y-10">
      {/* Live-subscription replay — the headline "push an Update, watch it fire" demo. */}
      <section className="space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Radio className="size-5 text-primary" aria-hidden="true" />
            <h2 className="text-xl font-semibold">Live subscription — replay</h2>
          </div>
          <div className="flex gap-2">
            <Button size="sm" onClick={play} disabled={playing && !done}>
              <Play className="size-4" aria-hidden="true" />
              {done ? "Replay" : playing ? "Playing…" : "Play"}
            </Button>
            <Button size="sm" variant="outline" onClick={reset} disabled={step === 0}>
              <RotateCcw className="size-4" aria-hidden="true" />
              Reset
            </Button>
          </div>
        </div>
        <p className="measure text-sm text-muted-foreground">
          A real SSE subscription, captured frame-by-frame. Press{" "}
          <strong className="text-foreground">Play</strong>: the stream opens and a{" "}
          <code className="font-mono">sequence&nbsp;0</code> full snapshot arrives, then a
          SPARQL <code className="font-mono">UPDATE</code> commits in another tab and the{" "}
          <em>same</em> stream pushes a <code className="font-mono">sequence&nbsp;1</code>{" "}
          incremental <code className="font-mono">addedResults</code> diff — the
          subscription firing on the write. The WebSocket{" "}
          <code className="font-mono">/subscriptions</code> path carries identical JSON.
        </p>
        <div className="space-y-2">
          {SUBSCRIPTION_TRANSCRIPT.map((frame, i) => (
            <div
              key={i}
              className={cn(
                "transition-opacity duration-300",
                i < step ? "opacity-100" : "opacity-0",
              )}
              aria-hidden={i >= step}
            >
              <TranscriptFrame frame={frame} active={i === step - 1} />
            </div>
          ))}
          {step === 0 && (
            <p className="rounded-lg border border-dashed bg-muted/20 p-4 text-center text-sm text-muted-foreground">
              Press Play to step through the captured stream.
            </p>
          )}
        </div>
      </section>

      {/* REST recipes — the protocol surface, each with verbatim captured I/O. */}
      <section className="space-y-4">
        <div className="flex items-center gap-2">
          <Terminal className="size-5 text-primary" aria-hidden="true" />
          <h2 className="text-xl font-semibold">Protocol recipes — captured curl I/O</h2>
        </div>
        <p className="measure text-sm text-muted-foreground">
          Every request below was run against a local{" "}
          <code className="font-mono">sparq-server</code> (default build) and its response
          recorded. The query/CONSTRUCT result bodies are deterministic — copy a recipe,
          point it at your own running endpoint over the same seed, and you get the same
          bytes. A few fields are inherently run-dependent (the{" "}
          <code className="font-mono">date:</code> header, and the{" "}
          <code className="font-mono">/metrics</code> request counters and histogram
          timings); those are shown as one representative capture.
        </p>
        <div className="grid gap-4 lg:grid-cols-2">
          {RECIPES.map((r) => (
            <RecipeCard key={r.id} recipe={r} />
          ))}
        </div>
      </section>
    </div>
  );
}
