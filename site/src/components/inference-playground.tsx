"use client";

// [OPUS-4.8] sq-0po6 — the live /surface/inference playground: load an ontology + facts
// (or an N3 rule document), forward-chain the closure in your tab via the tier-b W-reason
// wasm bundle, see the base-vs-entailed triple counts, then click an entailed triple to
// render its why() derivation proof tree. The default is the Socrates syllogism. Mirrors
// the SHACL playground's shape (examples → editor → run → result), but loads the SEPARATE
// reason bundle (src/lib/reason-wasm.ts) lazily so the landing page never pays for it.

import * as React from "react";
import { useHydrated } from "@/lib/use-hydrated";
import {
  Play,
  Loader2,
  Brain,
  CheckCircle2,
  Sparkles,
  HelpCircle,
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
  loadReasoner,
  prewarmReason,
  PROFILE_LABEL,
  type MaterializeStats,
  type ProofTree,
  type ReasoningProfile,
} from "@/lib/reason-wasm";
import {
  formatTriple,
  parseNTriples,
  parseProof,
  parseStats,
  proofRows,
  ruleLabel,
  shortenTerm,
  type Triple,
} from "@/lib/inference";
// [OPUS-4.8] sq-8uew — Turtle syntax-highlighting input editor (Turtle mode only).
import { RdfEditor } from "@/components/rdf-editor";
import {
  INFERENCE_EXAMPLES,
  type InferenceMode,
} from "@/data/inference-examples";

interface ReasonResult {
  /** Counts (profile mode only; N3 mode has no closure/base split). */
  stats: MaterializeStats | null;
  /** The NEWLY-entailed triples (profile mode) or the N3-derived ground triples. */
  entailed: Triple[];
  ms: number;
  mode: InferenceMode;
  profile: ReasoningProfile;
  /** The exact document reasoned over (so a why() click reasons the same input). */
  data: string;
}

type RunState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "result"; result: ReasonResult }
  | { kind: "error"; message: string };

type EngineState = "cold" | "warming" | "ready" | "error";

const DEFAULT = INFERENCE_EXAMPLES[0];
const PROFILES: ReasoningProfile[] = ["rdfs", "owl-rl"];

export function InferencePlayground() {
  const [data, setData] = React.useState(DEFAULT.data);
  const [mode, setMode] = React.useState<InferenceMode>(DEFAULT.mode);
  const [profile, setProfile] = React.useState<ReasoningProfile>(DEFAULT.profile);
  const [state, setState] = React.useState<RunState>({ kind: "idle" });
  const [engineState, setEngine] = React.useState<EngineState>("cold");
  const hydrated = useHydrated();
  const engine = hydrated && engineState === "cold" ? "warming" : engineState;
  const [activeExample, setActiveExample] = React.useState<string>(DEFAULT.id);
  // The triple whose proof is being shown (and the proof itself / loading flag).
  const [proof, setProof] = React.useState<{
    triple: Triple;
    tree: ProofTree | null;
    loading: boolean;
  } | null>(null);

  // Pre-warm the W-reason bundle on mount (off the render path) so the first "Reason"
  // pays no cold start. A failure resets the indicator; reasoning retries the load.
  React.useEffect(() => {
    let cancelled = false;
    prewarmReason()
      .then(() => {
        if (!cancelled) setEngine("ready");
      })
      .catch((e) => {
        if (cancelled) return;
        setEngine("error");
        toast.error("Reasoner failed to load", {
          description: e instanceof Error ? e.message : String(e),
        });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const run = React.useCallback(async () => {
    setState({ kind: "running" });
    setProof(null);
    try {
      const Reasoner = await loadReasoner();
      const t0 = performance.now();
      let stats: MaterializeStats | null = null;
      let entailed: Triple[];
      if (mode === "n3") {
        entailed = parseNTriples(Reasoner.reasonN3(data));
      } else {
        stats = parseStats(Reasoner.materializeStats(data, "turtle", profile));
        entailed = parseNTriples(Reasoner.entailed(data, "turtle", profile));
      }
      const ms = performance.now() - t0;
      setEngine("ready");
      setState({
        kind: "result",
        result: { stats, entailed, ms, mode, profile, data },
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setState({ kind: "error", message });
      toast.error("Reasoning failed", { description: message });
    }
  }, [data, mode, profile]);

  const explain = React.useCallback(
    async (triple: Triple, result: ReasonResult) => {
      // N3 rule reasoning has no profile-based proof tree (why() runs RDFS/OWL closure).
      if (result.mode === "n3") return;
      setProof({ triple, tree: null, loading: true });
      try {
        const Reasoner = await loadReasoner();
        if (typeof Reasoner.why !== "function") {
          throw new Error(
            "This W-reason bundle was built without the `explain` feature (no why binding).",
          );
        }
        const json = Reasoner.why(
          result.data,
          "turtle",
          result.profile,
          triple.subject,
          triple.predicate,
          triple.object,
        );
        setProof({ triple, tree: parseProof(json), loading: false });
      } catch (e) {
        setProof(null);
        toast.error("Proof failed", {
          description: e instanceof Error ? e.message : String(e),
        });
      }
    },
    [],
  );

  const selectExample = React.useCallback((id: string) => {
    const ex = INFERENCE_EXAMPLES.find((e) => e.id === id);
    if (!ex) return;
    setData(ex.data);
    setMode(ex.mode);
    setProfile(ex.profile);
    setActiveExample(id);
    setState({ kind: "idle" });
    setProof(null);
  }, []);

  const busy = state.kind === "running";

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <Brain className="size-4 text-primary" />
          Live reasoner
        </CardTitle>
        {/* [FABLE-5] sq-2av3s — the live-reasoner status badge reflects the W-reason wasm
            bundle load outcome (cold → warming → ready | error). In the lean-wasm VISUAL lane
            the reason bundle genuinely cannot load, so this region renders the
            "Reasoner failed — retries on reason" state and the load-error toast baked into the
            baseline, making the surface-inference screenshot drift. Mask this load-driven region
            at capture ("mask, don't chase", cf. the benchmarks provenance strip in #1801) so the
            baseline guards the static playground layout, not the runtime load outcome. The
            vrMasks() helper in e2e/visual/*.spec.ts masks every [data-vr-mask] node. */}
        <span data-vr-mask className="inline-flex">
          <EngineIndicator engine={engine} />
        </span>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap gap-1.5">
          {INFERENCE_EXAMPLES.map((ex) => (
            <Button
              key={ex.id}
              variant={activeExample === ex.id ? "default" : "outline"}
              size="sm"
              onClick={() => selectExample(ex.id)}
              title={ex.description}
            >
              {ex.label}
            </Button>
          ))}
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <ModeTabs mode={mode} onChange={setMode} />
          {mode === "profile" && (
            <div
              role="radiogroup"
              aria-label="Reasoning profile"
              className="inline-flex rounded-lg border bg-muted/40 p-0.5"
            >
              {PROFILES.map((p) => (
                <button
                  key={p}
                  type="button"
                  role="radio"
                  aria-checked={profile === p}
                  onClick={() => setProfile(p)}
                  className={cn(
                    "rounded-md px-2.5 py-1 text-xs font-medium transition-colors outline-none focus-visible:ring-3 focus-visible:ring-ring/40",
                    profile === p
                      ? "bg-background text-foreground shadow-sm"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {PROFILE_LABEL[p]}
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="space-y-1.5">
          <label
            htmlFor="reason-data"
            className="text-xs font-medium text-muted-foreground"
          >
            {mode === "n3"
              ? "N3 rules + facts"
              : "Ontology + facts (Turtle)"}
          </label>
          {/* [OPUS-4.8] sq-8uew — Turtle syntax highlighting for the Turtle ontology+facts
              input. N3 mode keeps the plain textarea: the Turtle lexer would mis-style
              N3-specific rule syntax (`{ } => @forAll`), so we only highlight pure Turtle. */}
          {mode === "n3" ? (
            <textarea
              id="reason-data"
              value={data}
              spellCheck={false}
              onChange={(e) => setData(e.target.value)}
              rows={12}
              className="w-full resize-y rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed outline-none focus-visible:ring-3 focus-visible:ring-ring/40"
            />
          ) : (
            <RdfEditor
              id="reason-data"
              ariaLabel="Ontology and facts (Turtle)"
              value={data}
              onChange={setData}
              rows={12}
            />
          )}
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <Button onClick={run} disabled={busy}>
            {busy ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Play className="size-4" />
            )}
            Reason
          </Button>
          <p aria-live="polite" className="text-xs text-muted-foreground">
            {state.kind === "result" &&
              `${state.result.entailed.length} entailed · ${state.result.ms.toFixed(1)} ms`}
            {state.kind === "running" && "Forward-chaining on the wasm reasoner…"}
            {state.kind === "idle" &&
              engine === "warming" &&
              "Pre-warming the W-reason bundle…"}
          </p>
        </div>

        <ResultPanel
          state={state}
          proof={proof}
          onExplain={explain}
        />
      </CardContent>
    </Card>
  );
}

function EngineIndicator({ engine }: { engine: EngineState }) {
  if (engine === "ready") {
    return (
      <Badge variant="success" aria-live="polite">
        <CheckCircle2 className="size-3" /> Reasoner ready
      </Badge>
    );
  }
  if (engine === "error") {
    return (
      <Badge variant="warning" aria-live="polite">
        Reasoner failed — retries on reason
      </Badge>
    );
  }
  return (
    <Badge variant="muted" aria-live="polite">
      <Loader2 className="size-3 animate-spin" /> Bundle loading…
    </Badge>
  );
}

function ModeTabs({
  mode,
  onChange,
}: {
  mode: InferenceMode;
  onChange: (m: InferenceMode) => void;
}) {
  const tabs: { value: InferenceMode; label: string }[] = [
    { value: "profile", label: "RDFS / OWL" },
    { value: "n3", label: "N3 rules" },
  ];
  return (
    <div
      role="tablist"
      aria-label="Reasoning mode"
      className="inline-flex rounded-lg border bg-muted/40 p-0.5"
    >
      {tabs.map((t) => (
        <button
          key={t.value}
          type="button"
          role="tab"
          aria-selected={mode === t.value}
          onClick={() => onChange(t.value)}
          className={cn(
            "rounded-md px-2.5 py-1 text-xs font-medium transition-colors outline-none focus-visible:ring-3 focus-visible:ring-ring/40",
            mode === t.value
              ? "bg-background text-foreground shadow-sm"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

function ResultPanel({
  state,
  proof,
  onExplain,
}: {
  state: RunState;
  proof: {
    triple: Triple;
    tree: ProofTree | null;
    loading: boolean;
  } | null;
  onExplain: (triple: Triple, result: ReasonResult) => void;
}) {
  if (state.kind === "error") {
    return (
      <pre className="overflow-x-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
        {state.message}
      </pre>
    );
  }
  if (state.kind !== "result") return null;
  const { result } = state;

  return (
    <div className="space-y-3">
      {result.stats && <CountSummary stats={result.stats} />}
      <EntailedList result={result} onExplain={onExplain} />
      {proof && <ProofView proof={proof} />}
    </div>
  );
}

function CountSummary({ stats }: { stats: MaterializeStats }) {
  const cells: { label: string; value: number; accent?: boolean }[] = [
    { label: "asserted (base)", value: stats.baseTriples },
    { label: "after closure", value: stats.closureTriples },
    { label: "entailed (added)", value: stats.entailed, accent: true },
  ];
  return (
    <div className="grid grid-cols-3 gap-2">
      {cells.map((c) => (
        <div
          key={c.label}
          className={cn(
            "rounded-lg border bg-muted/30 p-3 text-center",
            c.accent &&
              "bg-[color-mix(in_oklch,var(--success)_12%,transparent)] ring-1 ring-[var(--success)]/30",
          )}
        >
          <div
            className={cn(
              "text-xl font-semibold tabular-nums",
              c.accent ? "text-[var(--success)]" : "text-foreground",
            )}
          >
            {c.value}
          </div>
          <div className="text-[11px] text-muted-foreground">{c.label}</div>
        </div>
      ))}
    </div>
  );
}

function EntailedList({
  result,
  onExplain,
}: {
  result: ReasonResult;
  onExplain: (triple: Triple, result: ReasonResult) => void;
}) {
  if (result.entailed.length === 0) {
    return (
      <p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
        No new triples were entailed — the document is already deductively closed
        under {result.mode === "n3" ? "its rules" : PROFILE_LABEL[result.profile]}.
      </p>
    );
  }
  const canExplain = result.mode === "profile";
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 text-sm font-medium">
        <Sparkles className="size-4 text-[var(--success)]" aria-hidden="true" />
        Entailed triples
        <span className="text-xs font-normal text-muted-foreground">
          {canExplain ? "— click “why?” for the derivation" : ""}
        </span>
      </div>
      <ul className="space-y-1.5">
        {result.entailed.map((t, i) => (
          <li
            key={i}
            className="flex items-center justify-between gap-3 rounded-lg border bg-muted/30 px-3 py-2"
          >
            <code className="break-all font-mono text-[12.5px] text-foreground">
              {formatTriple(t)}
            </code>
            {canExplain && (
              <Button
                variant="outline"
                size="sm"
                className="shrink-0"
                onClick={() => onExplain(t, result)}
              >
                <HelpCircle className="size-3.5" aria-hidden="true" />
                why?
              </Button>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

function ProofView({
  proof,
}: {
  proof: {
    triple: Triple;
    tree: ProofTree | null;
    loading: boolean;
  };
}) {
  return (
    <div className="space-y-2 rounded-lg border bg-background p-3">
      <div className="text-sm font-medium">
        Why{" "}
        <code className="font-mono text-[12.5px] text-primary">
          {formatTriple(proof.triple)}
        </code>
        ?
      </div>
      {proof.loading ? (
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="size-3 animate-spin" /> Deriving the proof…
        </p>
      ) : proof.tree === null ? (
        <p className="text-xs text-muted-foreground">
          That triple is not entailed — no derivation.
        </p>
      ) : (
        <ProofTreeView tree={proof.tree} />
      )}
    </div>
  );
}

function ProofTreeView({ tree }: { tree: ProofTree }) {
  const rows = proofRows(tree);
  return (
    <ol className="space-y-1">
      {rows.map((row, i) => {
        const asserted = row.node.rule === "asserted";
        return (
          <li
            key={i}
            className="flex flex-wrap items-center gap-2 text-[12.5px]"
            style={{ paddingLeft: `${row.depth * 1.25}rem` }}
          >
            <Badge
              variant={asserted ? "muted" : "default"}
              className="font-mono text-[10.5px]"
            >
              {ruleLabel(row.node.rule)}
            </Badge>
            <code className="break-all font-mono text-foreground">
              {shortenTerm(row.node.conclusion[0])}{" "}
              {shortenTerm(row.node.conclusion[1])}{" "}
              {shortenTerm(row.node.conclusion[2])}
            </code>
            {row.repeated && (
              <span className="text-[11px] text-muted-foreground">
                (see above)
              </span>
            )}
          </li>
        );
      })}
    </ol>
  );
}
