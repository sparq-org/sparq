import type { Metadata } from "next";
import { ShieldCheck } from "lucide-react";

import { SurfaceContent } from "@/components/surface-content";
import { ShaclPlayground } from "@/components/shacl-playground";

export const metadata: Metadata = {
  title: "SHACL",
  description:
    "Validate RDF against SHACL shapes with the sparq engine — SHACL Core constraints, SHACL-SPARQL, custom constraint components, the W3C validation report, and a SHACL Compact Syntax view, live in your tab.",
};

// [OPUS-4.8] sq-vw3ax.8 — condensed to the scan-first template: demo first, one-line lead,
// ≤5 capabilities, single-sentence runs note + reproduce, caveats behind <details>, plus the
// universal README + SKILL.md links. The compact-syntax boundary detail moves to the SKILL.
export default function ShaclSurfacePage() {
  return (
    <SurfaceContent
      icon={ShieldCheck}
      title="SHACL"
      statement="Validate RDF data against shapes — SHACL Core, SHACL-SPARQL, the W3C validation report, and a live SHACL Compact Syntax view."
      tier="live"
      intro={
        <p>
          <code className="font-mono text-foreground">sparq-shacl</code> validates a
          graph against SHACL Core + SHACL-SPARQL constraints and returns a W3C
          validation report (conformance plus per-violation results); the playground
          above runs the validator in your tab via the SHACL-enabled wasm bundle that{" "}
          <code className="font-mono text-foreground">@sparq-org/sparq</code> ships.
        </p>
      }
      capabilities={[
        {
          title: "SHACL Core constraints",
          body: "class, datatype, cardinality, value ranges, node/property shapes, paths, plus and / or / not / xone, qualified & closed shapes.",
        },
        {
          title: "SHACL-SPARQL (§5.2)",
          body: "sh:sparql constraints expressed as SPARQL SELECT over the engine.",
        },
        {
          title: "Custom constraint components (§6)",
          body: "Define new sh:ConstraintComponent backed by SPARQL.",
        },
        {
          title: "W3C validation report",
          body: "Conformance boolean + sh:result violations, as Turtle or human text.",
        },
        {
          title: "SHACL Compact Syntax (in + out)",
          body: "Author shapes in the W3C compact notation (parsed in-tab to a shapes graph) and render any shapes graph back to compact form.",
        },
      ]}
      runsNote="Live in your browser tab — the validator is pure Rust over the engine compiled to the SHACL-enabled wasm bundle, so the conformance flag and per-violation report come back with no network round-trip."
      reproduce="cargo test -p sparq-shacl"
      caveat={
        <>
          <p>
            SHACL-SPARQL constraints rely on the engine&rsquo;s REGEX, which the SHACL
            bundle keeps (the lean SPARQL REPL bundle compiles it out), so{" "}
            <code className="font-mono">sh:sparql</code> validates in-tab too. The in-tab
            validator is sized for small documents (~10–100 triples); validate large
            graphs server-side via the <code className="font-mono">sparq-server</code>{" "}
            HTTP <code className="font-mono">validate</code> path.
          </p>
          <p>
            <strong className="text-foreground">SHACL Compact Syntax</strong> works both
            ways. The <em>input</em> direction (compact text &rarr; shapes) parses in-tab
            via the engine&rsquo;s <code className="font-mono">parseShaclCompact</code>{" "}
            binding &mdash; switch the shapes editor to <em>Compact</em> to author shapes
            in the terser notation and validate against them unchanged. The{" "}
            <em>display</em> direction (shapes &rarr; compact) is best-effort: the notation
            has no form for logical constraints or shape references, which the view lists
            explicitly rather than dropping.
          </p>
        </>
      }
      readmeHref="https://github.com/sparq-org/sparq/tree/main/crates/sparq-shacl"
      skillHref="https://github.com/sparq-org/sparq/blob/main/skills/shacl-validation/SKILL.md"
    >
      <ShaclPlayground />
    </SurfaceContent>
  );
}
