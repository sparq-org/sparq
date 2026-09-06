// [OPUS-4.8] sq-vz2v: Node / RDF-JS reference-engine adapter for the SHACL
// differential fuzzer (crates/sparq-shacl/tests/diff_fuzz.rs).
//
// A THIRD family of independent references alongside the pySHACL (sq-55c1) and
// Apache Jena (sq-evws) adapters. It is the SAME "report-cli" contract (bead
// sq-eifd): JSON request in, normalised JSON report out, so the Rust runner's
// comparison is engine-agnostic. Two RDF-JS SHACL validators are wired behind one
// `SHACL_DIFF_NODE_ENGINE` switch — both are Zazuko, MIT-licensed:
//   - "shacl-engine"        (https://github.com/rdf-ext/shacl-engine)
//   - "rdf-validate-shacl"  (https://github.com/zazuko/rdf-validate-shacl)
// A third engine catches bugs where sparq + pySHACL + Jena happen to agree but are
// all wrong, and — being JS over RDF-JS — it is the same family sparq's own
// @sparq-org/sparq WASM SHACL would slot into (the bonus JS-vs-JS lane noted on the
// bead), so this harness is the on-ramp for that.
//
// WHY .mjs run directly (no build step): Node runs an ES module file directly
// (`node node_shacl_adapter.mjs`), and both validators + the `n3` Turtle parser
// are pure JS. So the differential ships ONLY source; the npm packages come from
// a `node_modules` the CI lane installs and the Rust runner points at via
// NODE_PATH (mirroring how the Jena adapter is handed a `-cp` classpath).
//
// CONTRACT (stdin -> stdout) — identical to tests/diff_fuzz/pyshacl_adapter.py
// and tests/diff_fuzz/JenaShaclAdapter.java:
//   stdin  : a JSON object {"data": "<turtle>", "shapes": "<turtle>"}
//   stdout : {"conforms": bool,
//             "violations": [{"focus": str|null, "component": str|null,
//                             "path": str|null}, ...]}
//            — `focus`/`component` are full IRIs (or "_:bnode" for blank nodes,
//            which have no cross-graph identity); `path` is the bare predicate IRI
//            for a simple path, the "_:path" sentinel for any complex/blank-rooted
//            path, or null when the result carries no path. Exactly the shape the
//            Rust runner's RefReport deserialises and `ref_keys` normalises.
//   exit   : 0 on a produced report (even non-conforming); non-zero only on an
//            adapter/engine ERROR (so the runner distinguishes "engine says X" from
//            "engine could not run"), with a diagnostic on stderr.
//
// Reads ONE request and emits ONE report (one Node process per case, mirroring the
// pySHACL/Jena adapters' one-runtime-per-case isolation).
//
// `--selftest`: exercises the sh:detail exclusion + path/term normalisation on a
// synthetic RDF-JS validation report (no validator run, so it is independent of
// which engine/version is installed). Exit 0 on pass, non-zero with a diagnostic
// on fail. Mirrors the Jena adapter's --selftest and the pyshacl adapter's intent.

// `@zazuko/env-node` is a full RDF-JS environment (data-model + dataset +
// clownface): the ONE factory that drives BOTH validators — shacl-engine wants a
// `.literal`/`.namedNode` data factory, rdf-validate-shacl additionally wants
// `.clownface`. Using one factory keeps the two engines on identical term
// construction, so a difference can only come from validation logic.
import rdf from "@zazuko/env-node";
import { Parser } from "n3";

const SH = "http://www.w3.org/ns/shacl#";
const RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

// Which RDF-JS validator to drive. Default `shacl-engine` (actively maintained,
// SHACL-AF support); `rdf-validate-shacl` selectable for a second JS opinion.
function nodeEngineName() {
  return (process.env.SHACL_DIFF_NODE_ENGINE || "shacl-engine").trim();
}

function parseTurtle(ttl) {
  // n3's Parser.parse(string) is synchronous; collect quads into a fresh RDF-JS
  // dataset that both validators accept.
  const quads = new Parser({ format: "text/turtle" }).parse(ttl);
  return rdf.dataset(quads);
}

// Minimal NamedNode factory for dataset.match() predicates (avoids pulling a
// second data-factory dependency; n3/@rdfjs terms compare by termType+value).
function namedNode(iri) {
  return {
    termType: "NamedNode",
    value: iri,
    equals(o) {
      return !!o && o.termType === "NamedNode" && o.value === iri;
    },
  };
}

// --- term normalisation: the SAME keys the pySHACL / Jena adapters emit. -------

// A graph-independent key for a term: blank nodes collapse to "_:bnode" (no stable
// cross-graph identity — the tolerance every adapter + the Rust runner share);
// named nodes render to their IRI; literals (only used in diagnostics) to a stable
// label. null/undefined → null (a genuinely-absent field, distinct from a blank
// node — the Rust `ref_keys` maps it to the `<missing-focus>` sentinel).
function termKey(term) {
  if (term == null) return null;
  switch (term.termType) {
    case "BlankNode":
      return "_:bnode";
    case "NamedNode":
      return term.value;
    case "Literal":
      return `"${term.value}"`;
    default:
      return String(term.value);
  }
}

// An sh:resultPath term → the comparable string the other adapters emit: the bare
// predicate IRI for a simple (NamedNode) path, else the "_:path" sentinel for any
// complex / blank-rooted path (sequence/inverse/alternative/* are blank-node
// rooted), or null when there is no path.
function pathKey(term) {
  if (term == null) return null;
  if (term.termType === "NamedNode") return term.value;
  return "_:path";
}

// --- report normalisation over an RDF-JS validation-report dataset. ------------

// Both validators expose a `.dataset` of the RDF report (rdf-validate-shacl via
// `report.dataset`; shacl-engine via `report.dataset` too). We normalise straight
// off the RDF so we depend only on the standard SHACL report vocabulary, not on
// either library's bespoke JS result objects.
//
// [OPUS-4.8] (mirrors sq-0hj7 / sq-vsqr) `sh:detail` exclusion: a sh:ValidationResult
// reachable only as the OBJECT of an `sh:detail` is a NESTED sub-result for a failed
// inner constraint (SHACL §3.6.2, non-normative "MAY"). sparq keeps those in
// `ValidationResult::details`, NOT in the top-level `report.results`. Diffing nested
// sub-results would compare two engines' OPTIONAL detail policies, not a real
// disagreement — so we drop any result that is the object of an sh:detail, exactly
// as the pySHACL and Jena adapters do.
function normaliseDataset(dataset) {
  const p = (suffix) => SH + suffix;

  // Nodes that are the object of some sh:detail (to exclude as nested sub-results).
  const nested = new Set();
  for (const q of dataset.match(null, namedNode(p("detail")), null)) {
    nested.add(q.object.value);
  }

  const firstObject = (subj, predIri) => {
    for (const q of dataset.match(subj, namedNode(predIri), null)) return q.object;
    return null;
  };

  // `sh:conforms` lives on the sh:ValidationReport node; if absent (some engines
  // omit it) derive it from the presence of any top-level result.
  let conforms = null;
  for (const q of dataset.match(null, namedNode(p("conforms")), null)) {
    conforms = q.object.value === "true";
  }

  const violations = [];
  const seenResults = new Set();
  for (const q of dataset.match(null, namedNode(RDF_TYPE), namedNode(p("ValidationResult")))) {
    const res = q.subject;
    if (seenResults.has(res.value)) continue;
    seenResults.add(res.value);
    if (nested.has(res.value)) continue; // nested sh:detail sub-result — excluded
    violations.push({
      focus: termKey(firstObject(res, p("focusNode"))),
      component: termKey(firstObject(res, p("sourceConstraintComponent"))),
      path: pathKey(firstObject(res, p("resultPath"))),
    });
  }

  if (conforms == null) conforms = violations.length === 0;
  return { conforms, violations };
}

// --- engine drivers. -----------------------------------------------------------

async function validateWithShaclEngine(dataTtl, shapesTtl) {
  const { Validator } = await import("shacl-engine");
  const data = parseTurtle(dataTtl);
  const shapes = parseTurtle(shapesTtl);
  const validator = new Validator(shapes, { factory: rdf });
  const report = await validator.validate({ dataset: data });
  return normaliseDataset(report.dataset);
}

async function validateWithRdfValidateShacl(dataTtl, shapesTtl) {
  const mod = await import("rdf-validate-shacl");
  const SHACLValidator = mod.default || mod;
  const data = parseTurtle(dataTtl);
  const shapes = parseTurtle(shapesTtl);
  const validator = new SHACLValidator(shapes, { factory: rdf });
  const report = await validator.validate(data);
  return normaliseDataset(report.dataset);
}

async function run(dataTtl, shapesTtl) {
  const engine = nodeEngineName();
  switch (engine) {
    case "shacl-engine":
      return validateWithShaclEngine(dataTtl, shapesTtl);
    case "rdf-validate-shacl":
      return validateWithRdfValidateShacl(dataTtl, shapesTtl);
    default:
      throw new Error(
        `unknown SHACL_DIFF_NODE_ENGINE='${engine}' (want shacl-engine | rdf-validate-shacl)`,
      );
  }
}

// --- --selftest: prove the sh:detail exclusion + path/term normalisation. ------

function selfTest() {
  // Synthetic report dataset: a top-level result carrying a nested sh:detail to an
  // inner result. normaliseDataset must emit EXACTLY the top-level violation; the
  // nested sub-result is excluded. A regression that dropped the exclusion would
  // surface the inner result as a second violation here.
  const blank = (id) => ({ termType: "BlankNode", value: id });
  const lit = (v) => ({ termType: "Literal", value: v });
  const quad = (s, predIri, o) => ({
    subject: s,
    predicate: namedNode(predIri),
    object: o,
    graph: { termType: "DefaultGraph", value: "" },
  });
  const report = blank("report");
  const top = blank("top");
  const inner = blank("inner");
  const fX = namedNode("http://example.org/x");
  const pP = namedNode("http://example.org/p");
  const comp = namedNode(SH + "MinCountConstraintComponent");
  const innerComp = namedNode(SH + "DatatypeConstraintComponent");

  const quads = [
    quad(report, RDF_TYPE, namedNode(SH + "ValidationReport")),
    quad(report, SH + "conforms", lit("false")),
    // top-level result
    quad(top, RDF_TYPE, namedNode(SH + "ValidationResult")),
    quad(top, SH + "focusNode", fX),
    quad(top, SH + "resultPath", pP),
    quad(top, SH + "sourceConstraintComponent", comp),
    quad(top, SH + "detail", inner), // nested → must be excluded
    // nested (detail) result — MUST NOT appear at top level
    quad(inner, RDF_TYPE, namedNode(SH + "ValidationResult")),
    quad(inner, SH + "focusNode", fX),
    quad(inner, SH + "sourceConstraintComponent", innerComp),
  ];

  const out = normaliseDataset(rdf.dataset(quads));
  const ok =
    out.conforms === false &&
    out.violations.length === 1 &&
    out.violations[0].focus === "http://example.org/x" &&
    out.violations[0].path === "http://example.org/p" &&
    out.violations[0].component === SH + "MinCountConstraintComponent";
  if (!ok) {
    process.stderr.write(
      "node_shacl_adapter selftest FAIL: sh:detail exclusion / normalisation; got " +
        JSON.stringify(out) +
        "\n",
    );
    return false;
  }
  return true;
}

async function main() {
  if (process.argv.length === 3 && process.argv[2] === "--selftest") {
    process.exit(selfTest() ? 0 : 3);
  }
  let req;
  try {
    const chunks = [];
    for await (const c of process.stdin) chunks.push(c);
    req = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch (e) {
    process.stderr.write(`node_shacl_adapter: bad request JSON: ${e}\n`);
    process.exit(2);
  }
  let out;
  try {
    out = await run(req.data, req.shapes);
  } catch (e) {
    process.stderr.write(
      `node_shacl_adapter: engine error (${nodeEngineName()}): ${e && e.stack ? e.stack : e}\n`,
    );
    process.exit(1);
  }
  process.stdout.write(JSON.stringify(out) + "\n");
}

main();
