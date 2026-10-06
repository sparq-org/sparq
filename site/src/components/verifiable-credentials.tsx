"use client";

// [OPUS-4.8] sq-3p0z (#822) — the Verifiable-Credentials import-and-query workbench.
//
// The maintainer's ask (#822, research/gui-design.md §2b): "import VCs (drag-drop or URL) and
// run SPARQL queries over them." A W3C Verifiable Credential is an RDF document (a JSON-LD doc
// that maps to RDF, or already Turtle / N-Quads), so the base capability — load it into a graph
// and query it — is JUST the existing in-tab WASM ingest path, and is `live` today. This
// component is the IMPORT SURFACE: a drag-drop zone, a file picker, a paste box, and a URL
// fetch, plus a credentials list showing each imported VC's recognised envelope, issuer and
// subject — then the shared SPARQL editor + result table running over the union of every
// imported credential, entirely in your browser tab.
//
// HONESTY (the load-bearing distinction). *Querying* over VC triples is `live`. This surface
// does NOT cryptographically verify a VC's signature, nor produce any zero-knowledge proof —
// that is the separate research-grade ZK estate (sparq-zk), which is internally re-audited and
// NOT externally audited (sq-qhy4). SD-JWT-VC / JWT-VC envelopes are RECOGNISED (so the user
// is told they are JWT-wrapped and need decoding/verification first), never silently mis-parsed.
// The page never presents "verified" or any ZK property as a guarantee.

import * as React from "react";
import {
  FileLock2,
  Upload,
  Link2,
  ClipboardPaste,
  Play,
  Loader2,
  X,
  CheckCircle2,
  AlertTriangle,
  ShieldQuestion,
  Trash2,
  FilePlus2,
} from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import { withBasePath } from "@/lib/base-path";
import { SparqlEditor } from "@/components/sparql-editor";
import {
  loadSparq,
  loadIntoStore,
  storeToNQuads,
  datasetSize,
  extractTable,
  termValue,
  type SparqlResults,
  type WasmStore,
} from "@/lib/sparq-wasm";
import { classifyQueryForm } from "@/lib/repl-dataset";
import {
  SAMPLE_CREDENTIALS,
  SAMPLE_QUERIES,
  VC_FORMAT_OPTIONS,
  vcFormatFromName,
  looksLikeJwtVc,
  type VcFormat,
} from "@/data/verifiable-credentials";

// ── Model ───────────────────────────────────────────────────────────────────────────────────

/** One imported credential: its source, the format it parsed as, and a few derived facts. */
interface ImportedCredential {
  /** Unique key (timestamp + counter) so React lists are stable across re-imports. */
  key: string;
  label: string;
  source: string;
  format: VcFormat;
  /** How it arrived (drives the small origin tag). */
  origin: "sample" | "file" | "paste" | "url";
  /** Triples this credential contributes (counted at parse time). */
  triples: number;
  /** Issuer IRI, if the credential states one (cred:issuer). */
  issuer: string | null;
  /** Subject id (credentialSubject id / a Person identifier), if present. */
  subject: string | null;
}

type QueryState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "select"; results: SparqlResults; ms: number }
  | { kind: "boolean"; value: boolean; ms: number }
  | { kind: "graph"; ntriples: string; triples: number; ms: number }
  | { kind: "error"; message: string };

// Lightweight introspection queries to surface a credential's issuer + subject in the list.
// Run against an EPHEMERAL store holding only that one credential, so the facts are per-VC.
const ISSUER_QUERY =
  "PREFIX cred: <https://www.w3.org/2018/credentials#> SELECT ?i WHERE { ?c cred:issuer ?i } LIMIT 1";
// The W3C VC 2.0 (v2 context) subject predicate expands to the credentials/v2 IRI; the older
// 2018 vocabulary uses cred:credentialSubject. We also fall back to a schema.org identifier.
const SUBJECT_QUERY = `PREFIX cred: <https://www.w3.org/2018/credentials#>
PREFIX cred2: <https://www.w3.org/ns/credentials/issuer-dependent#>
PREFIX sch: <https://schema.org/>
SELECT ?s WHERE {
  { ?c <https://www.w3.org/2018/credentials#credentialSubject> ?s }
  UNION { ?c <https://www.w3.org/ns/credentials#credentialSubject> ?s }
  UNION { ?subject sch:identifier ?s }
} LIMIT 1`;

/** First binding's single value, or null — for the per-credential introspection queries. */
function firstValue(store: WasmStore, query: string): string | null {
  try {
    const parsed = JSON.parse(store.query(query)) as SparqlResults;
    const row = parsed.results?.bindings?.[0];
    if (!row) return null;
    const term = Object.values(row)[0];
    return termValue(term) ?? null;
  } catch {
    return null;
  }
}

// ── Component ───────────────────────────────────────────────────────────────────────────────

let importCounter = 0;
function nextKey(): string {
  importCounter += 1;
  return `vc-${Date.now()}-${importCounter}`;
}

export function VerifiableCredentials() {
  const [creds, setCreds] = React.useState<ImportedCredential[]>([]);
  const [query, setQuery] = React.useState<string>(SAMPLE_QUERIES[0].sparql);
  const [state, setState] = React.useState<QueryState>({ kind: "idle" });
  const [seeded, setSeeded] = React.useState(false);

  // Parse one RDF/JSON-LD document into an ephemeral store and derive its per-VC facts. Throws
  // a clear Error on a parse failure so the caller can surface it (and abort the import).
  const describe = React.useCallback(
    async (
      label: string,
      source: string,
      format: VcFormat,
      origin: ImportedCredential["origin"],
    ): Promise<ImportedCredential> => {
      const Store = await loadSparq();
      const store = loadIntoStore(Store, source, format);
      return {
        key: nextKey(),
        label,
        source,
        format,
        origin,
        triples: datasetSize(store),
        issuer: firstValue(store, ISSUER_QUERY),
        subject: firstValue(store, SUBJECT_QUERY),
      };
    },
    [],
  );

  // Seed the two sample credentials on first mount so the surface shows it working before the
  // user imports their own. Best-effort: a wasm load hiccup leaves the list empty (the import
  // affordances still work), it never throws into render.
  React.useEffect(() => {
    if (seeded) return;
    let cancelled = false;
    setSeeded(true);
    (async () => {
      const out: ImportedCredential[] = [];
      for (const s of SAMPLE_CREDENTIALS) {
        try {
          out.push(await describe(s.label, s.source, s.format, "sample"));
        } catch {
          // Skip a sample that fails to parse; the rest still seed.
        }
      }
      if (!cancelled) setCreds(out);
    })();
    return () => {
      cancelled = true;
    };
  }, [seeded, describe]);

  // Add a document to the credentials list (parsing + introspection happen here). Rejects a
  // JWT-shaped envelope with an explanation rather than mis-parsing it as RDF.
  const addDocument = React.useCallback(
    async (
      label: string,
      source: string,
      format: VcFormat,
      origin: ImportedCredential["origin"],
    ) => {
      const trimmed = source.trim();
      if (!trimmed) return;
      if ((format === "jsonld" || origin === "paste") && looksLikeJwtVc(trimmed)) {
        toast.error("That looks like a JWT-wrapped VC", {
          description:
            "SD-JWT-VC / JWT-VC envelopes are not plain RDF — they must be decoded (and their proof verified) before their claims can be queried as triples. That is the separate, not-externally-audited crypto estate.",
        });
        return;
      }
      try {
        const cred = await describe(label, trimmed, format, origin);
        setCreds((prev) => [...prev, cred]);
        toast.success(`Imported ${label}`, {
          description: `${cred.triples.toLocaleString()} triple${cred.triples === 1 ? "" : "s"} added — query it below.`,
        });
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        // The lean in-tab bundle has no remote-context loader, so a JSON-LD VC that references
        // its @context only by URL fails here. Explain the fix rather than leak the raw error.
        const remoteContext = /remote context|LoadDocumentCallback/i.test(message);
        toast.error("Could not parse the credential", {
          description: remoteContext
            ? "This JSON-LD credential references a remote @context by URL, which the in-tab engine does not fetch. Inline the @context terms, pre-expand the document, or import it as Turtle / N-Quads."
            : message,
        });
      }
    },
    [describe],
  );

  const removeCredential = React.useCallback((key: string) => {
    setCreds((prev) => prev.filter((c) => c.key !== key));
  }, []);

  const clearAll = React.useCallback(() => {
    setCreds([]);
    setState({ kind: "idle" });
  }, []);

  // Run the SPARQL query over the UNION of every imported credential. Each credential's RDF is
  // re-serialised to N-Quads and concatenated, then loaded once via loadDataset — so named
  // graphs survive and the query sees all credentials together, exactly like the REPL's merge.
  const runQuery = React.useCallback(async () => {
    if (creds.length === 0) {
      toast.error("No credentials imported", {
        description: "Import a VC (drag-drop, paste or URL) before querying.",
      });
      return;
    }
    setState({ kind: "running" });
    try {
      const Store = await loadSparq();
      // Build the combined store from every credential's N-Quads.
      let combined = "";
      for (const c of creds) {
        const one = loadIntoStore(Store, c.source, c.format);
        combined += storeToNQuads(one) + "\n";
      }
      const store = loadIntoStore(Store, combined, "nquads");

      const form = classifyQueryForm(query);
      const t0 = performance.now();
      if (form === "construct" || form === "describe") {
        const ntriples = store.queryQuads(query).trim();
        const triples = ntriples === "" ? 0 : ntriples.split("\n").length;
        setState({ kind: "graph", ntriples, triples, ms: performance.now() - t0 });
        return;
      }
      if (form === "update") {
        throw new Error(
          "This surface is read-only over imported credentials — SPARQL Update is not supported here. Use SELECT / ASK / CONSTRUCT / DESCRIBE.",
        );
      }
      const json = store.query(query);
      const ms = performance.now() - t0;
      const parsed = JSON.parse(json) as SparqlResults;
      if (typeof parsed.boolean === "boolean") {
        setState({ kind: "boolean", value: parsed.boolean, ms });
      } else {
        setState({ kind: "select", results: parsed, ms });
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setState({ kind: "error", message });
      toast.error("Query failed", { description: message });
    }
  }, [creds, query]);

  const totalTriples = creds.reduce((n, c) => n + c.triples, 0);

  return (
    <div className="space-y-6">
      <div className="grid gap-6 lg:grid-cols-2">
        <ImportPanel onAdd={addDocument} />
        <CredentialsList
          creds={creds}
          totalTriples={totalTriples}
          onRemove={removeCredential}
          onClear={clearAll}
        />
      </div>

      <QueryPanel
        query={query}
        onQuery={setQuery}
        onRun={runQuery}
        running={state.kind === "running"}
        disabled={creds.length === 0}
      />

      <ResultPanel state={state} />

      <HonestyNote />
    </div>
  );
}

// ── Import panel: drag-drop + file + paste + URL ──────────────────────────────────────────────

type ImportTab = "drop" | "paste" | "url";

function ImportPanel({
  onAdd,
}: {
  onAdd: (
    label: string,
    source: string,
    format: VcFormat,
    origin: ImportedCredential["origin"],
  ) => void;
}) {
  const [tab, setTab] = React.useState<ImportTab>("drop");
  const [dragOver, setDragOver] = React.useState(false);
  const fileInputRef = React.useRef<HTMLInputElement>(null);

  // Paste tab.
  const [pasteText, setPasteText] = React.useState("");
  const [pasteFormat, setPasteFormat] = React.useState<VcFormat>("jsonld");
  // URL tab.
  const [url, setUrl] = React.useState("");
  const [fetching, setFetching] = React.useState(false);

  const ingestFiles = React.useCallback(
    async (files: FileList | File[]) => {
      for (const file of Array.from(files)) {
        try {
          const text = await file.text();
          onAdd(file.name, text, vcFormatFromName(file.name), "file");
        } catch (e) {
          toast.error(`Could not read ${file.name}`, {
            description: e instanceof Error ? e.message : String(e),
          });
        }
      }
    },
    [onAdd],
  );

  const onDrop = React.useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const dt = e.dataTransfer;
      if (dt.files && dt.files.length > 0) {
        void ingestFiles(dt.files);
        return;
      }
      // A drag of selected text (not a file) — treat it as a pasted document.
      const text = dt.getData("text/plain");
      if (text.trim()) {
        onAdd("dropped text", text, looksLikeJwtVc(text) ? "jsonld" : "turtle", "paste");
      }
    },
    [ingestFiles, onAdd],
  );

  const importUrl = React.useCallback(async () => {
    const target = url.trim();
    if (!target) return;
    setFetching(true);
    try {
      const res = await fetch(target);
      if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`);
      const body = await res.text();
      const fmt = formatFromContentType(res.headers.get("content-type")) ?? vcFormatFromName(target);
      onAdd(urlLabel(target), body, fmt, "url");
      setUrl("");
    } catch (e) {
      toast.error("Fetch failed", {
        description: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setFetching(false);
    }
  }, [url, onAdd]);

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <Upload className="size-4 text-primary" />
          Import a Verifiable Credential
        </CardTitle>
        <Badge variant="success">Live · in your tab</Badge>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex border-b text-xs" role="tablist" aria-label="Import method">
          <ImportTabButton id="drop" current={tab} onSelect={setTab} icon={FilePlus2} label="Drag-drop / file" />
          <ImportTabButton id="paste" current={tab} onSelect={setTab} icon={ClipboardPaste} label="Paste" />
          <ImportTabButton id="url" current={tab} onSelect={setTab} icon={Link2} label="URL" />
        </div>

        {tab === "drop" && (
          <div className="space-y-2">
            <div
              data-vc-dropzone
              onDragOver={(e) => {
                e.preventDefault();
                setDragOver(true);
              }}
              onDragLeave={() => setDragOver(false)}
              onDrop={onDrop}
              onClick={() => fileInputRef.current?.click()}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  fileInputRef.current?.click();
                }
              }}
              aria-label="Drop a credential file here or click to choose one"
              className={cn(
                "flex cursor-pointer flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed p-8 text-center transition-colors",
                dragOver
                  ? "border-primary bg-primary/5 text-foreground"
                  : "border-foreground/15 text-muted-foreground hover:border-primary/50 hover:text-foreground",
              )}
            >
              <Upload className="size-6" aria-hidden />
              <p className="text-sm font-medium">
                Drop a VC file here, or click to choose
              </p>
              <p className="text-xs">
                JSON-LD VC, Turtle, N-Quads, TriG or N-Triples — parsed in your browser.
              </p>
              <p className="text-[11px] text-muted-foreground/80">
                In-tab JSON-LD needs an inline <code className="font-mono">@context</code> (no
                remote context fetch); or import the VC as Turtle / N-Quads.
              </p>
            </div>
            {/* [FABLE-5] sq-mcsbp — the sr-only file input is a real focusable form
                element, so axe's `label` rule (critical) requires it to carry its own
                accessible name; the aria-label on the visible dropzone div does not name
                this input. Give the input an explicit aria-label. */}
            <input
              ref={fileInputRef}
              type="file"
              aria-label="Choose credential file(s) to import (JSON-LD, Turtle, N-Quads, TriG or N-Triples)"
              accept=".jsonld,.json,.ttl,.turtle,.nq,.nquads,.trig,.nt,.ntriples,application/ld+json,text/turtle"
              multiple
              className="sr-only"
              onChange={(e) => {
                if (e.target.files) void ingestFiles(e.target.files);
                e.target.value = "";
              }}
            />
          </div>
        )}

        {tab === "paste" && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <label htmlFor="vc-paste-format" className="text-xs font-medium text-muted-foreground">
                Format
              </label>
              <select
                id="vc-paste-format"
                value={pasteFormat}
                onChange={(e) => setPasteFormat(e.target.value as VcFormat)}
                className="rounded-md border bg-background px-2 py-1 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
              >
                {VC_FORMAT_OPTIONS.map((f) => (
                  <option key={f.value} value={f.value}>
                    {f.label}
                  </option>
                ))}
              </select>
            </div>
            <textarea
              value={pasteText}
              onChange={(e) => setPasteText(e.target.value)}
              placeholder={'{ "@context": "https://www.w3.org/ns/credentials/v2", "type": ["VerifiableCredential"], … }'}
              spellCheck={false}
              className="h-40 w-full resize-y rounded-md border bg-background px-3 py-2 font-mono text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
            />
            <Button
              size="sm"
              disabled={!pasteText.trim()}
              onClick={() => {
                onAdd("pasted credential", pasteText, pasteFormat, "paste");
                setPasteText("");
              }}
            >
              <FilePlus2 className="size-4" /> Add pasted credential
            </Button>
          </div>
        )}

        {tab === "url" && (
          <div className="space-y-2">
            <p className="text-xs text-muted-foreground">
              Fetch a credential document from a URL (the format is auto-detected from the
              served <code className="font-mono">Content-Type</code> or the extension). The
              host must allow the cross-origin request.
            </p>
            <div className="flex gap-2">
              {/* [FABLE-5] accessible name for the URL input (label critical, sq-0rbfn) */}
              <input
                type="url"
                inputMode="url"
                aria-label="Credential URL"
                placeholder="https://example.org/credential.jsonld"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                className="w-full rounded-md border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
              />
              <Button size="sm" disabled={!url.trim() || fetching} onClick={importUrl}>
                {fetching ? <Loader2 className="size-4 animate-spin" /> : <Link2 className="size-4" />}
                Fetch
              </Button>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function ImportTabButton({
  id,
  current,
  onSelect,
  icon: Icon,
  label,
}: {
  id: ImportTab;
  current: ImportTab;
  onSelect: (t: ImportTab) => void;
  icon: typeof FilePlus2;
  label: string;
}) {
  const active = id === current;
  return (
    <button
      role="tab"
      aria-selected={active}
      data-vc-import-tab={id}
      onClick={() => onSelect(id)}
      className={cn(
        "flex flex-1 items-center justify-center gap-1.5 border-b-2 px-3 py-2 font-medium",
        active
          ? "border-primary text-foreground"
          : "border-transparent text-muted-foreground hover:text-foreground",
      )}
    >
      <Icon className="size-3.5" /> {label}
    </button>
  );
}

// Map a response Content-Type to the engine format (kept local — small + VC-specific).
function formatFromContentType(contentType: string | null): VcFormat | undefined {
  if (!contentType) return undefined;
  const mime = contentType.split(";")[0].trim().toLowerCase();
  const map: Record<string, VcFormat> = {
    "application/ld+json": "jsonld",
    "application/json": "jsonld",
    "text/turtle": "turtle",
    "application/n-quads": "nquads",
    "application/trig": "trig",
    "application/n-triples": "ntriples",
  };
  return map[mime];
}

/** A short, readable label for a URL (last path segment, or the host). */
function urlLabel(url: string): string {
  try {
    const u = new URL(url);
    const last = u.pathname.split("/").filter(Boolean).pop();
    return last || u.host;
  } catch {
    return url;
  }
}

// ── Credentials list ──────────────────────────────────────────────────────────────────────────

function CredentialsList({
  creds,
  totalTriples,
  onRemove,
  onClear,
}: {
  creds: ImportedCredential[];
  totalTriples: number;
  onRemove: (key: string) => void;
  onClear: () => void;
}) {
  return (
    <Card data-vc-list>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <FileLock2 className="size-4 text-primary" />
          Imported credentials
        </CardTitle>
        <div className="flex items-center gap-2">
          <Badge variant="muted" className="tabular" data-vc-count>
            {creds.length} · {totalTriples.toLocaleString()} triples
          </Badge>
          {creds.length > 0 && (
            <Button variant="ghost" size="icon" aria-label="Clear all credentials" onClick={onClear}>
              <Trash2 className="size-4" />
            </Button>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-2">
        {creds.length === 0 ? (
          <p className="rounded-lg border bg-muted/30 p-4 text-sm text-muted-foreground">
            No credentials imported yet. Drag-drop a VC file, paste one, or fetch it from a URL —
            then query it below.
          </p>
        ) : (
          <ul className="space-y-2">
            {creds.map((c) => (
              <li
                key={c.key}
                data-vc-cred
                className="flex items-start gap-3 rounded-lg border bg-card p-3"
              >
                <FileLock2 className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    <span className="truncate text-sm font-medium">{c.label}</span>
                    <Badge variant="outline" className="h-4 px-1.5 text-[10px] uppercase">
                      {c.format}
                    </Badge>
                    <span className="text-[11px] text-muted-foreground">
                      {c.origin} · {c.triples.toLocaleString()} triples
                    </span>
                  </div>
                  {c.issuer && (
                    <p className="truncate font-mono text-[11px] text-muted-foreground">
                      issuer: {c.issuer}
                    </p>
                  )}
                  {c.subject && (
                    <p className="truncate font-mono text-[11px] text-muted-foreground">
                      subject: {c.subject}
                    </p>
                  )}
                </div>
                <button
                  onClick={() => onRemove(c.key)}
                  aria-label={`Remove ${c.label}`}
                  className="shrink-0 text-muted-foreground hover:text-foreground"
                >
                  <X className="size-4" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

// ── Query panel ─────────────────────────────────────────────────────────────────────────────

function QueryPanel({
  query,
  onQuery,
  onRun,
  running,
  disabled,
}: {
  query: string;
  onQuery: (q: string) => void;
  onRun: () => void;
  running: boolean;
  disabled: boolean;
}) {
  return (
    <Card>
      <CardHeader className="space-y-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Play className="size-4 text-primary" />
          Query the imported credentials
        </CardTitle>
        <div className="flex flex-wrap gap-1.5" role="group" aria-label="Example queries">
          {SAMPLE_QUERIES.map((q) => (
            <Button
              key={q.id}
              variant="outline"
              size="sm"
              title={q.blurb}
              onClick={() => onQuery(q.sparql)}
            >
              {q.label}
            </Button>
          ))}
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        <SparqlEditor value={query} onChange={onQuery} rows={9} ariaLabel="SPARQL query over imported credentials" />
        <div className="flex items-center gap-3">
          <Button onClick={onRun} disabled={running || disabled} data-vc-run>
            {running ? <Loader2 className="size-4 animate-spin" /> : <Play className="size-4" />}
            Run query
          </Button>
          <p className="text-xs text-muted-foreground">
            SELECT / ASK / CONSTRUCT / DESCRIBE over the union of every imported credential,
            executed in your browser tab.
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

// ── Result panel ────────────────────────────────────────────────────────────────────────────

function ResultPanel({ state }: { state: QueryState }) {
  if (state.kind === "idle") return null;
  if (state.kind === "running") {
    return (
      <Card>
        <CardContent className="flex items-center gap-2 pt-5 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" /> Running the query in your tab…
        </CardContent>
      </Card>
    );
  }
  if (state.kind === "error") {
    return (
      <Card className="ring-destructive/30">
        <CardContent className="pt-5">
          <pre
            data-vc-result="error"
            className="overflow-x-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive"
          >
            {state.message}
          </pre>
        </CardContent>
      </Card>
    );
  }
  return (
    <Card data-vc-result={state.kind}>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="text-base">Result</CardTitle>
        <Badge variant="muted" className="tabular">
          {state.ms.toFixed(1)} ms · in your tab
        </Badge>
      </CardHeader>
      <CardContent>
        {state.kind === "boolean" ? (
          <div
            className={cn(
              "rounded-lg p-4 text-center text-2xl font-semibold",
              state.value
                ? "bg-[color-mix(in_oklch,var(--success)_14%,transparent)] text-[var(--success)]"
                : "bg-muted text-muted-foreground",
            )}
          >
            {state.value ? "true" : "false"}
          </div>
        ) : state.kind === "graph" ? (
          <div className="space-y-2">
            <p className="text-xs text-muted-foreground">
              {state.triples.toLocaleString()} triple{state.triples === 1 ? "" : "s"} constructed.
            </p>
            <pre className="max-h-96 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed">
              {state.ntriples || "(empty graph)"}
            </pre>
          </div>
        ) : (
          <SelectTable results={state.results} />
        )}
      </CardContent>
    </Card>
  );
}

function SelectTable({ results }: { results: SparqlResults }) {
  const table = React.useMemo(() => extractTable(results), [results]);
  if (table.rows.length === 0) {
    return (
      <p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
        No solutions.
      </p>
    );
  }
  return (
    <div className="max-h-96 overflow-auto rounded-lg border">
      <table className="w-full text-left text-sm">
        <thead className="sticky top-0 bg-muted/80 backdrop-blur">
          <tr>
            {table.vars.map((v) => (
              <th key={v} className="px-3 py-2 font-medium">
                ?{v}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {table.rows.map((row, i) => (
            <tr key={i} className="border-t">
              {row.map((cell, j) => (
                <td key={table.vars[j]} className="px-3 py-1.5 font-mono text-[12.5px]">
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ── Honesty: import + query is live; verification / ZK is the separate, unaudited estate ───────

function HonestyNote() {
  return (
    <Card className="ring-[var(--warning)]/30">
      <CardHeader className="flex-row items-center gap-2 space-y-0">
        <ShieldQuestion className="size-5 text-[var(--warning)]" />
        <CardTitle className="text-base">Imported &amp; queryable — not verified</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 text-sm text-muted-foreground">
        <p className="flex items-start gap-2">
          <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-[var(--success)]" />
          <span>
            <strong className="text-foreground">What is live.</strong> Importing a Verifiable
            Credential and running SPARQL over its claims is the real sparq engine, compiled to
            WebAssembly and running entirely in your browser tab — a VC is RDF (JSON-LD that maps
            to RDF, or already Turtle / N-Quads), so this is just the existing ingest path.
            Nothing is sent to a server.
          </span>
        </p>
        <p className="flex items-start gap-2">
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-[var(--warning)]" />
          <span>
            <strong className="text-foreground">What is NOT done here.</strong> This surface does{" "}
            <strong className="text-foreground">not</strong> cryptographically verify a
            credential&rsquo;s signature, check its issuer or revocation status, nor produce any
            zero-knowledge proof about it. Querying a claim is distinct from{" "}
            <em>verifying</em> it. JWT-wrapped envelopes (SD-JWT-VC / JWT-VC) are recognised but
            not parsed — they must be decoded and their proof verified first.
          </span>
        </p>
        <p>
          VC <em>verification</em> and zero-knowledge proofs over credentials are the separate
          sparq ZK estate (<code className="font-mono">sparq-zk</code>). That estate is{" "}
          <strong className="text-foreground">research-grade</strong>: internally re-audited but{" "}
          <strong className="text-foreground">not externally audited</strong> — any timing or
          gate count there is an indicative engineering number, not an audited cryptographic
          guarantee, and external accredited-cryptographer sign-off is pending (
          <code className="font-mono">sq-qhy4</code>). Treat any future &ldquo;verified&rdquo;
          affordance accordingly. See the{" "}
          {/* [FABLE-5] sq-mcsbp — persistent underline (not hover-only): this link sits inline
              inside a muted card paragraph where its --primary teal is only ~1.03:1 against the
              surrounding muted text, so axe's link-in-text-block (serious) requires a non-colour
              distinguisher. */}
          <a
            className="text-primary underline underline-offset-4"
            href={withBasePath("/showcase/zk-car-hire")}
          >
            ZK car-hire demo
          </a>{" "}
          for a real (research-grade) zero-knowledge proof over credential data.
        </p>
      </CardContent>
    </Card>
  );
}
