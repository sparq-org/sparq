// [OPUS-5] sq-gum8.16 (paper factory F4) — the PUBLICATION-BOUNDARY source gate for measured
// timings.
//
// WHY THIS EXISTS: `site/papers/_lib/timing.typ` promises that a measured wall-clock number can
// only reach a page WITH its provenance. Typst cannot enforce that on its own — it has no
// private module bindings, so every top-level `#let` in that lib (including the parsed dataset
// and the internal `_trec` lookup) is importable, and any paper can also just `json()` the
// generated file itself. Both are bare-value escape hatches around `headline_timing()`.
//
// So the invariant is enforced HERE, mechanically, over the paper sources the factory is about
// to compile: a paper may import ONLY the provenance-rendering entry points from the timing
// lib, may not import it as a whole module, may not touch its internals, and may not load data
// from disk at all. Anything else FAILS THE BUILD (build-papers.mjs) — the fail-closed
// direction, because the alternative is publishing a number stripped of the machine, corpus,
// aggregate and commit that make it meaningful.
//
// WHY THE RULE IS ON THE CONSTRUCT, NOT THE PATH: denylisting the string `paper-timing
// .generated` is bypassed by any expression that never spells it — `json("/src/data/paper-" +
// "timing.generated.json")` names no internal binding and imports no timing module, yet loads
// the same dataset. So the boundary is drawn at Typst's data-loading builtins themselves
// (`json`/`read`/`csv`/…) and at non-literal `#import` paths: a paper source may not contain
// one, full stop, which makes the path expression irrelevant. The two evidence-library
// implementations that must load data are allowed exactly one pinned expression each
// (`EXEMPT` / `AUDITED_LOADER_LINES`), so the exemption cannot widen unnoticed.
//
// WHY REFLECTION IS REFUSED TOO: a lexical gate cannot see a name assembled at runtime —
// `#let grab = eval("j" + "son", mode: "code")` yields the loader while spelling neither `json`
// nor the timing path, and `std.json(…)` reaches it through the standard-library namespace
// instead of the bare binding. Neither can be matched by looking for a loader name, so the
// reflective constructs THEMSELVES (`eval`, `std`) are refused in a paper source, in code
// position — `eval(P)` as SPARQL notation in prose or a raw span is untouched, since neither
// evaluates. So a paper cannot manufacture a loader out of fragments it never names, and the
// only modules it may pull in are the local `_lib/` ones this same scan covers.
//
// HONEST SCOPE: this is a SOURCE-TEXT check over `site/papers/**/*.typ`. It bounds the paper
// surface the factory compiles; it is not a Typst-level capability system — a structural
// boundary in which paper compilation simply cannot address the raw timing JSON would be
// stronger. Within its lexical means it is fail-closed: loader calls, loader aliases, computed
// import paths, and the reflective constructs that could manufacture any of them are all
// refused. It says nothing about whether a rendered number is FRAMED honestly (that stays the
// Stage-5 claims↔evidence review in `skills/academic-paper/SKILL.md`).

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

// The only bindings a paper may import from the timing lib. `timing_keys` yields record KEYS
// (never values), so it cannot leak a bare number; the other two always render provenance.
export const ALLOWED_TIMING_IMPORTS = new Set([
  "headline_timing",
  "timing_provenance",
  "timing_keys",
]);

// The lib itself and its compile self-check are the implementation of the boundary, not
// consumers of it, so they are the only files exempt from the scan.
const EXEMPT = new Set(["_lib/timing.typ", "_lib/timing-selfcheck.typ"]);

// Typst's data-loading builtins. Naming one in a paper source is a route to a raw value that
// never passes through a provenance-rendering accessor, whatever path expression it is given.
const DATA_LOADERS = ["json", "csv", "yaml", "toml", "xml", "cbor", "read"];
const LOADER_LIST = DATA_LOADERS.join("|");
// A call: `json(…)`, `read (…)`. The lookbehind keeps field accesses (`x.read(…)`) and longer
// identifiers out; a loader reached that way still needs a bare loader call for its bytes.
const LOADER_CALL = new RegExp(`(?<![A-Za-z0-9_.])(${LOADER_LIST})\\s*\\(`, "g");
// The same construct, renamed: `#let j = json` … `#j("/src/data/…")`. The call form no longer
// sees it once the binding is aliased, so the binding position is matched too.
const LOADER_ALIAS = new RegExp(
  `#let\\s+[A-Za-z_][A-Za-z0-9_]*\\s*=\\s*(${LOADER_LIST})\\b\\s*(?!\\()`,
  "g",
);
// Reflective routes to a loader that never spell its name: `eval` (code evaluation) and `std`
// (the standard-library namespace, which `LOADER_CALL`'s lookbehind deliberately skips over as a
// field access). Papers discuss `eval(P)` as SPARQL notation in prose, so the token is matched
// only in code position — directly after `#`, `=`, `(`, `,`, `{`, `;` or `:` — which is where
// Typst would actually evaluate it, and never after a space or a backtick as prose does.
const REFLECTIVE = ["eval", "std"];
const REFLECTIVE_CONSTRUCT = new RegExp(
  `[#=(,{;:]\\s*(${REFLECTIVE.join("|")})(?![A-Za-z0-9_])`,
  "g",
);
// The evidence libraries genuinely have to load data — that is what they are. Each is allowed
// the ONE audited expression it needs, pinned as a whole line, so any other loader call in them
// (a constructed path included) still fails and the exemption cannot grow silently.
const AUDITED_LOADER_LINES = new Map([
  ["_lib/bench.typ", [/^#let evidence = json\(bytes\(sys\.inputs\.data\)\)$/]],
]);

// `#import`/`#include` with a path that is not a plain string literal — the constructed-path
// bypass applied to module loading, which `TIMING_IMPORT` below (a literal-path matcher) would
// otherwise not see.
const ANY_IMPORT = /#(import|include)\s+([^\n]*)/g;
const LITERAL_IMPORT_PATH = /^"[^"]*"\s*(?::|$)/;

const GENERATED_TIMING_FILE = /paper-timing\.generated/;
// `#import "…/timing.typ"` / `#include "…"`, capturing the optional `: a, b` binding list.
const TIMING_IMPORT = /#(import|include)\s+"([^"]*\btiming\.typ)"\s*(?::([^\n]*))?/g;
// Internal (convention-private) bindings of the lib. Importing or naming any of them in a paper
// is an attempt to read a record without rendering its provenance.
const INTERNALS = /\b(_timing_data|_trec|_envelope|_fmt_us|_host_short)\b/;

function walkTyp(dir, root = dir, out = []) {
  if (!existsSync(dir)) return out;
  for (const entry of readdirSync(dir).sort()) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) walkTyp(p, root, out);
    else if (entry.endsWith(".typ")) out.push(relative(root, p));
  }
  return out;
}

// Typst allows `#import "x": a, b as c, *`. Take the name actually bound FROM the module (the
// part before `as`), and treat a wildcard as importing everything, internals included.
function importedNames(list) {
  return list
    .split(",")
    .map((s) => s.trim().split(/\s+as\s+/)[0].trim())
    .filter(Boolean);
}

// The whole source line a match sits on, trimmed — the unit `AUDITED_LOADER_LINES` pins.
function lineAt(src, index) {
  const start = src.lastIndexOf("\n", index) + 1;
  const end = src.indexOf("\n", index);
  return src.slice(start, end === -1 ? src.length : end).trim();
}

/**
 * Audit every paper source under `papersDir` for a bypass of the forced-provenance boundary.
 * Returns `{ scanned, problems }`; `problems` is empty iff the boundary holds.
 */
export function auditTimingSources(papersDir) {
  const problems = [];
  const files = walkTyp(papersDir).filter((f) => !EXEMPT.has(f.split(/[\\/]/).join("/")));
  for (const rel of files) {
    const src = readFileSync(join(papersDir, rel), "utf8");
    if (GENERATED_TIMING_FILE.test(src)) {
      problems.push(
        `${rel}: reads the derived timing file (paper-timing.generated.json) directly. A ` +
          "measured number may only render through headline_timing()/timing_provenance(), " +
          "which print the provenance beside it.",
      );
    }
    if (INTERNALS.test(src)) {
      problems.push(
        `${rel}: names an internal binding of _lib/timing.typ (_timing_data / _trec / ...). ` +
          "Those return raw records; use headline_timing(key) or timing_provenance(key).",
      );
    }
    // No data loading in a paper source, whatever path expression it is handed. (Same path
    // normalisation as the EXEMPT filter, so the audited allowance keys hold on Windows too.)
    const audited = AUDITED_LOADER_LINES.get(rel.split(/[\\/]/).join("/")) ?? [];
    for (const re of [LOADER_CALL, LOADER_ALIAS]) {
      for (const m of src.matchAll(re)) {
        const line = lineAt(src, m.index);
        if (audited.some((ok) => ok.test(line))) continue;
        problems.push(
          `${rel}: uses the data loader '${m[1]}' (\`${line}\`). A paper source may not load ` +
            "data from disk at all — a constructed or aliased path reaches the derived timing " +
            "file just as a literal one does. Render measured numbers via headline_timing(key) " +
            "/ timing_provenance(key).",
        );
      }
    }
    // ...and may not manufacture one reflectively either, which no loader-name rule can see.
    for (const m of src.matchAll(REFLECTIVE_CONSTRUCT)) {
      const line = lineAt(src, m.index);
      if (audited.some((ok) => ok.test(line))) continue;
      problems.push(
        `${rel}: uses the reflective construct '${m[1]}' (\`${line}\`). Code evaluation and the ` +
          "std namespace yield a data loader without ever naming it, so a paper source may not " +
          "use them in code position. Render measured numbers via headline_timing(key) / " +
          "timing_provenance(key); for SPARQL notation in prose, write `eval(P)` as raw text.",
      );
    }
    // ...and an import path must be a literal, so the check below can actually see it.
    for (const m of src.matchAll(ANY_IMPORT)) {
      const [, form, arg] = m;
      if (!LITERAL_IMPORT_PATH.test(arg.trim())) {
        problems.push(
          `${rel}: \`#${form}\` with a non-literal path (\`${arg.trim()}\`). A computed module ` +
            "path defeats the timing-import rules; write the path as a plain string literal.",
        );
      }
    }
    for (const m of src.matchAll(TIMING_IMPORT)) {
      const [, form, path, list] = m;
      if (list === undefined) {
        problems.push(
          `${rel}: \`#${form} "${path}"\` pulls in the whole timing module, which exposes the ` +
            "raw dataset. Import only: " + [...ALLOWED_TIMING_IMPORTS].join(", ") + ".",
        );
        continue;
      }
      for (const name of importedNames(list)) {
        if (!ALLOWED_TIMING_IMPORTS.has(name)) {
          problems.push(
            `${rel}: imports '${name}' from ${path}. Only ` +
              [...ALLOWED_TIMING_IMPORTS].join(", ") +
              " may be imported — every other binding can yield a timing without its provenance.",
          );
        }
      }
    }
  }
  return { scanned: files, problems };
}
