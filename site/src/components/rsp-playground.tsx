"use client";

// [OPUS-4.8] sq-11zy — the live /surface/streaming-rsp playground. The browser tab drives
// the logical clock: pick an example, then "Push next reading" feeds one timestamped
// triple at a time (or "Run stream" replays them all), and each push that crosses a
// window boundary fires the closed windows — their AVG(?v) / COUNT / ISTREAM table comes
// back per close. A RANGE / STEP slider lets you re-shape the window (tumbling vs sliding)
// and re-run. Everything runs in-tab via the separate, lazy-loaded W-rsp wasm bundle
// (registerRsp -> Rsp.select/push/flush); nothing is sent to a server.

import * as React from "react";
import { useHydrated } from "@/lib/use-hydrated";
import {
  Play,
  SkipForward,
  RotateCcw,
  Loader2,
  Radio,
  CheckCircle2,
  Flag,
} from "lucide-react";
import { toast } from "sonner";

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
  prewarmRsp,
  registerRsp,
  type WasmRsp,
} from "@/lib/sparq-rsp-wasm";
import {
  parseClosedWindows,
  windowCells,
  windowLabel,
  type ClosedWindow,
} from "@/lib/rsp-window";
import { RSP_EXAMPLES, type RspExample } from "@/data/rsp-examples";

type EngineState = "cold" | "warming" | "ready" | "error";

const DEFAULT = RSP_EXAMPLES[0];

export function RspPlayground() {
  const [example, setExample] = React.useState<RspExample>(DEFAULT);
  const [engineState, setEngine] = React.useState<EngineState>("cold");
  const hydrated = useHydrated();
  const engine = hydrated && engineState === "cold" ? "warming" : engineState;
  // Window controls (seeded from the example, then user-tunable via the sliders).
  const [range, setRange] = React.useState(DEFAULT.range);
  const [step, setStep] = React.useState(DEFAULT.step);
  // Stream replay state: the live handle, how many readings we've pushed, the windows
  // fired so far, the late-dropped counter, and whether we've flushed (end-of-stream).
  const [handle, setHandle] = React.useState<WasmRsp | null>(null);
  const [cursor, setCursor] = React.useState(0);
  const [windows, setWindows] = React.useState<ClosedWindow[]>([]);
  const [lateDropped, setLateDropped] = React.useState(0);
  const [flushed, setFlushed] = React.useState(false);
  const configuration = React.useMemo(() => ({ example, range, step }), [example, range, step]);
  const [registered, setRegistered] = React.useState<typeof configuration | null>(null);
  const [resetting, setResetting] = React.useState(false);
  const [registrationError, setError] = React.useState<string | null>(null);
  const busy = resetting || (hydrated && registered !== configuration);
  const error = registered === configuration ? registrationError : null;

  // Pre-warm the (separate) streaming wasm bundle on mount so the first push pays no
  // cold start. A failure resets the indicator; the first register retries the load.
  React.useEffect(() => {
    let cancelled = false;
    prewarmRsp()
      .then(() => {
        if (!cancelled) setEngine("ready");
      })
      .catch((e) => {
        if (cancelled) return;
        setEngine("error");
        toast.error("Streaming engine failed to load", {
          description: e instanceof Error ? e.message : String(e),
        });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Re-register the continuous query with the current window when the example, RANGE or
  // STEP changes. Re-registering resets the stream (a fresh logical clock), so we also
  // clear the replay state. The wasm handle is stateful; we keep one live per config.
  const reset = React.useCallback((isCancelled: () => boolean = () => false) => {
    const { example, range, step } = configuration;
    return registerRsp(example.query, range, step, example.maxDelay, example.r2s).then(
      (h) => {
        if (isCancelled()) return;
        setEngine("ready");
        setError(null);
        setHandle(h);
        setCursor(0);
        setWindows([]);
        setLateDropped(0);
        setFlushed(false);
        setRegistered(configuration);
        setResetting(false);
      },
      (e) => {
        if (isCancelled()) return;
        const message = e instanceof Error ? e.message : String(e);
        setError(message);
        setHandle(null);
        setRegistered(configuration);
        setResetting(false);
        toast.error("Could not register the continuous query", { description: message });
      },
    );
  }, [configuration]);

  // Register the external wasm query; only its completion callbacks update React state.
  React.useEffect(() => {
    let cancelled = false;
    void reset(() => cancelled);
    return () => { cancelled = true; };
  }, [reset]);

  const selectExample = React.useCallback((id: string) => {
    const ex = RSP_EXAMPLES.find((e) => e.id === id);
    if (!ex) return;
    setExample(ex);
    setRange(ex.range);
    setStep(ex.step);
  }, []);

  // Push ONE reading and absorb whatever windows it closed.
  const pushOne = React.useCallback(() => {
    if (!handle || cursor >= example.readings.length) return;
    try {
      const r = example.readings[cursor];
      const closed = parseClosedWindows(handle.push(r.s, r.p, r.o, r.ts));
      setWindows((prev) => [...prev, ...closed]);
      setLateDropped(handle.lateDropped());
      setCursor((c) => c + 1);
      setError(null);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      toast.error("Push failed", { description: message });
    }
  }, [handle, cursor, example.readings]);

  // Replay every remaining reading, then flush (end-of-stream) to close the tail window.
  const runAll = React.useCallback(() => {
    if (!handle) return;
    try {
      const closed: ClosedWindow[] = [];
      for (let i = cursor; i < example.readings.length; i++) {
        const r = example.readings[i];
        closed.push(...parseClosedWindows(handle.push(r.s, r.p, r.o, r.ts)));
      }
      closed.push(...parseClosedWindows(handle.flush()));
      setWindows((prev) => [...prev, ...closed]);
      setLateDropped(handle.lateDropped());
      setCursor(example.readings.length);
      setFlushed(true);
      setError(null);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      toast.error("Stream run failed", { description: message });
    }
  }, [handle, cursor, example.readings]);

  // Flush only (close the open tail window without more pushes).
  const flush = React.useCallback(() => {
    if (!handle || flushed) return;
    try {
      const closed = parseClosedWindows(handle.flush());
      setWindows((prev) => [...prev, ...closed]);
      setLateDropped(handle.lateDropped());
      setFlushed(true);
      setError(null);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      toast.error("Flush failed", { description: message });
    }
  }, [handle, flushed]);

  const tumbling = step >= range;
  const remaining = example.readings.length - cursor;
  const done = cursor >= example.readings.length;

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <Radio className="size-4 text-primary" />
          Live RSP-QL stream processor
        </CardTitle>
        <EngineIndicator engine={engine} />
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap gap-1.5">
          {RSP_EXAMPLES.map((ex) => (
            <Button
              key={ex.id}
              variant={example.id === ex.id ? "default" : "outline"}
              size="sm"
              onClick={() => selectExample(ex.id)}
              title={ex.description}
            >
              {ex.label}
            </Button>
          ))}
        </div>

        <p className="text-sm text-muted-foreground">{example.description}</p>

        <div className="space-y-1.5">
          <label className="text-xs font-medium text-muted-foreground">
            Continuous SELECT
          </label>
          <pre className="overflow-x-auto rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed">
            {example.query}
          </pre>
        </div>

        {/* RANGE / STEP sliders — re-shape the window (tumbling vs sliding) and re-run. */}
        <div className="grid gap-4 sm:grid-cols-2">
          <WindowSlider
            id="rsp-range"
            label="RANGE"
            help="window width (logical ticks)"
            value={range}
            min={5}
            max={120}
            step={5}
            onChange={setRange}
          />
          <WindowSlider
            id="rsp-step"
            label="STEP"
            help={tumbling ? "≥ RANGE ⇒ tumbling" : "< RANGE ⇒ sliding"}
            value={step}
            min={5}
            max={120}
            step={5}
            onChange={setStep}
          />
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <Button onClick={pushOne} disabled={busy || done || !handle}>
            <SkipForward className="size-4" />
            Push next reading
          </Button>
          <Button
            variant="outline"
            onClick={runAll}
            disabled={busy || done || !handle}
          >
            <Play className="size-4" />
            Run stream
          </Button>
          <Button
            variant="outline"
            onClick={flush}
            disabled={busy || flushed || !handle}
          >
            <Flag className="size-4" />
            Flush
          </Button>
          <Button variant="ghost" onClick={() => {
            setResetting(true);
            setError(null);
            void reset();
          }} disabled={busy}>
            <RotateCcw className="size-4" />
            Reset
          </Button>
          <p aria-live="polite" className="text-xs text-muted-foreground">
            {busy
              ? "Registering on the wasm engine…"
              : `${cursor} / ${example.readings.length} readings pushed · ${windows.length} window${windows.length === 1 ? "" : "s"} fired${lateDropped ? ` · ${lateDropped} late-dropped` : ""}`}
          </p>
        </div>

        {/* The next reading queued for the clock — makes "the tab is the clock" concrete. */}
        {!done && handle && (
          <p className="text-xs text-muted-foreground">
            Next push:{" "}
            <code className="font-mono text-foreground">
              {example.readings[cursor].s} {example.readings[cursor].p}{" "}
              {example.readings[cursor].o}
            </code>{" "}
            at <span className="font-mono text-foreground">ts={example.readings[cursor].ts}</span>{" "}
            ({remaining} remaining)
          </p>
        )}

        {error && (
          <pre className="overflow-x-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
            {error}
          </pre>
        )}

        <WindowList windows={windows} flushed={flushed && done} />
      </CardContent>
    </Card>
  );
}

function WindowSlider({
  id,
  label,
  help,
  value,
  min,
  max,
  step,
  onChange,
}: {
  id: string;
  label: string;
  help: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
}) {
  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between">
        <label htmlFor={id} className="text-xs font-medium text-muted-foreground">
          {label}{" "}
          <span className="font-mono text-foreground">{value}</span>
        </label>
        <span className="text-[11px] text-muted-foreground">{help}</span>
      </div>
      <input
        id={id}
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-full accent-[var(--primary)]"
      />
    </div>
  );
}

function EngineIndicator({ engine }: { engine: EngineState }) {
  if (engine === "ready") {
    return (
      <Badge variant="success" aria-live="polite">
        <CheckCircle2 className="size-3" /> Engine ready
      </Badge>
    );
  }
  if (engine === "error") {
    return (
      <Badge variant="warning" aria-live="polite">
        Engine failed — retries on register
      </Badge>
    );
  }
  return (
    <Badge variant="muted" aria-live="polite">
      <Loader2 className="size-3 animate-spin" /> Engine loading…
    </Badge>
  );
}

function WindowList({
  windows,
  flushed,
}: {
  windows: ClosedWindow[];
  flushed: boolean;
}) {
  if (windows.length === 0) {
    return (
      <p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
        No windows fired yet — push readings until the logical clock crosses a window
        boundary, or hit “Run stream”.
      </p>
    );
  }
  return (
    <div className="space-y-2" data-testid="rsp-windows">
      {windows.map((w, i) => (
        <WindowCard key={i} window={w} />
      ))}
      {flushed && (
        <p className="text-xs text-muted-foreground">
          Stream flushed — every window up to the last timestamp is closed.
        </p>
      )}
    </div>
  );
}

function WindowCard({ window: w }: { window: ClosedWindow }) {
  const { vars, rows } = windowCells(w);
  return (
    <div className="rounded-lg border bg-muted/30 p-3 text-sm" data-testid="rsp-window">
      <div className="mb-2 flex items-center gap-2">
        <Badge variant="muted" className="font-mono text-[11px]">
          {windowLabel(w)}
        </Badge>
        <span className="text-xs text-muted-foreground">
          {rows.length === 0
            ? "empty window"
            : `${rows.length} row${rows.length === 1 ? "" : "s"}`}
        </span>
      </div>
      {rows.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          Fired with no solutions (the watermark jumped this window).
        </p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full border-collapse font-mono text-[12.5px]">
            <thead>
              <tr className="text-left text-muted-foreground">
                {vars.map((v) => (
                  <th key={v} className="border-b px-2 py-1 font-medium">
                    ?{v}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, ri) => (
                <tr key={ri}>
                  {row.map((cell, ci) => (
                    <td
                      key={ci}
                      className={cn(
                        "border-b px-2 py-1 text-foreground",
                        cell === "" && "text-muted-foreground",
                      )}
                    >
                      {cell === "" ? "—" : cell}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
