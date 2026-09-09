import type { Metadata } from "next";
import { Boxes } from "lucide-react";

import { SurfaceContent } from "@/components/surface-content";
import { JavascriptWasmDemo } from "@/components/javascript-wasm-demo";
import { EsmImportSnippet } from "./esm-import-snippet";
import { withBasePath } from "@/lib/base-path";

export const metadata: Metadata = {
  title: "JavaScript / WASM",
  description:
    "Use the sparq engine from JavaScript/TypeScript via @sparq-org/sparq — an idiomatic RDF/JS surface (SparqStore + a named Dataset DatasetCore entry, importable from a <script type=module>) over the ~886 KB WebAssembly build, in Node or the browser.",
};

// [OPUS-4.8] sq-vw3ax.8 — condensed to the scan-first template: demo first, one-line lead,
// ≤5 capabilities, single-sentence runs note + reproduce, caveats behind <details>, plus the
// universal README + SKILL.md links. The full API tour lives in the SKILL. The ~886 KB figure
// is a build-output artifact size (not a benchmark) and is kept.
export default function JavascriptWasmSurfacePage() {
  return (
    <SurfaceContent
      icon={Boxes}
      title="JavaScript / WASM"
      statement="The @sparq-org/sparq RDF/JS API — the Rust engine compiled to a single ~886 KB wasm artifact."
      tier="live"
      intro={
        <p>
          <code className="font-mono text-foreground">@sparq-org/sparq</code> wraps the Rust
          triplestore + SPARQL engine — compiled to a single ~886 KB (≈314 KB gzipped)
          wasm artifact — in an idiomatic{" "}
          {/* [FABLE-5] persistent underline: distinguishable without color (link-in-text-block, sq-0rbfn) */}
          <a
            className="text-primary underline underline-offset-4"
            href="https://rdf.js.org/"
            target="_blank"
            rel="noopener noreferrer"
          >
            RDF/JS
          </a>{" "}
          surface that runs unchanged in Node &ge; 18 and the browser; this very site
          uses it for every live demo, and the panels below run against one seeded store
          in your tab.
        </p>
      }
      capabilities={[
        {
          title: "SparqStore (RDF/JS)",
          body: "fromString / fromCompressed, query() yielding Map-like Bindings, idiomatic terms, plus the raw Store class for SPARQL-JSON strings.",
        },
        {
          title: "Dataset — RDF/JS DatasetCore",
          body: "Named ESM entry: add / delete / has / match / size / iterate, lazily wasm-initialised; .store drops to the full SPARQL surface.",
        },
        {
          title: "Streaming cursors",
          body: "Iterate large SELECT results in batches without materialising the whole table.",
        },
        {
          title: "RDF/JS match() / countQuads() / count()",
          body: "The standard Source interface, plus a solution count read from the index without building every binding.",
        },
        {
          title: "applyDelta / SPARQL Update",
          body: "Apply quad-level deltas or SPARQL Update to mutate the store in place.",
        },
      ]}
      runsNote="This is the engine — every live demo on this site calls this API; in Node it loads the same wasm binary, and in the browser it streams it on first use, never on import."
      reproduce="npm i @sparq-org/sparq"
      caveat={
        <p>
          The <code className="font-mono">SparqStore.query()</code> wrapper covers
          SELECT/ASK; CONSTRUCT / DESCRIBE come via{" "}
          <code className="font-mono">queryQuads()</code> (RDF/JS quads),{" "}
          <code className="font-mono">queryQuadsString()</code> (N-Triples) and{" "}
          <code className="font-mono">queryQuadsStream()</code> — drop to the raw{" "}
          <code className="font-mono">Store</code> only to skip term materialisation. The
          lean bundle omits REGEX/REPLACE and the wall-clock query budget (see the SPARQL
          surface), trading a smaller download for those native-only features.
        </p>
      }
      readmeHref="https://github.com/sparq-org/sparq/tree/main/crates/sparq-wasm"
      skillHref="https://github.com/sparq-org/sparq/blob/main/skills/javascript-wasm/SKILL.md"
      links={[
        { href: withBasePath("/app/"), label: "Open the SPARQL workbench", external: true },
        {
          href: "https://www.npmjs.com/package/@sparq-org/sparq",
          label: "@sparq-org/sparq on npm",
          external: true,
        },
      ]}
    >
      <JavascriptWasmDemo />
      <EsmImportSnippet />
    </SurfaceContent>
  );
}
