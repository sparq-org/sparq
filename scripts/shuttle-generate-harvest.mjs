#!/usr/bin/env node
// [OPUS-5] sq-tonhr.12 — INTERIM Shuttle "generate mode" + differential harvest.
//
// WHAT THIS IS. rdf-shuttle's spec (spec/SHUTTLE.md §9) derives a third artifact
// from a grammar — a conformance-pair generator (`--emit tests`) producing
// coverage-directed (document, expected-quads) pairs plus provably-negative
// syntax tests from the LL tables. That mode is NOT IMPLEMENTED upstream (there
// is no generator source file and no CLI flag; verified at the pin recorded by
// `research/shuttle-generate-mode-harvest-2026-07.md`). This script is the
// INTERIM stand-in that makes the improvement thesis MEASURABLE now — "generated
// corpora catch corner cases hand-written parsers miss" — without waiting for the
// upstream implementation.
//
// WHAT IT IS NOT — read before quoting any number it prints:
//   * Positives are NOT valid-by-construction. The real generate mode runs the
//     relation with nothing ground, so every document is semantically valid by
//     construction. Here documents are derived from the grammar's production/
//     alternative structure with SAMPLED terminals, then FILTERED by parsing them
//     with the grammar's own generated parser (the oracle). Expected quads are
//     the oracle's output, so the oracle is trusted — a bug in it is invisible to
//     this harness and a divergence it reports may be the oracle's fault. Every
//     divergence needs human triage before it is filed as anyone's bug.
//   * Negatives are NOT provably outside the language. LL-table mutants require
//     the compiled parse tables; here mutants are token-level edits KEPT ONLY IF
//     the oracle rejects them ("oracle-rejected", a weaker property).
//   * Coverage is over (production x alternative) and `@covers` labels only — not
//     the spec's (production x alternative x token-boundary bucket x print-guard)
//     product.
//
// USAGE
//   node scripts/shuttle-generate-harvest.mjs --shuttle <rdf-shuttle checkout> \
//        [--grammar turtle12] [--seed 7] [--count 400] [--mutants 400] \
//        [--out <dir>] [--compare oxigraph:<path/to/oxigraph/node.js>] \
//        [--report <file.json>]
//   node scripts/shuttle-generate-harvest.mjs --shuttle <dir> --self-test
//
// Node lives ONLY in scripts/ (same rule as scripts/regen-shuttle-parsers.sh):
// never in `cargo build` or the Rust test suite. `--out` writes a corpus the
// sq-tonhr.2 Rust harness can consume unchanged — `<out>/manifest.ttl` names every
// document via `mf:action`, which is what
// `crates/sparq-conformance/src/differential.rs::run_suite_actions` walks;
// `run_dir` reads the same tree with a `.ttl` filter. Nothing is written into the
// repo: the corpus is reproducible from `--seed`, so it is not committed.
//
// `--self-test` is the non-vacuity proof: it runs the whole pipeline against
// deliberately seeded MUTANT comparators (one drops a quad, one accepts
// everything) and FAILS unless the harness reports a divergence for each. Run it
// after touching any comparison code.

import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

/* ------------------------------------------------------------------ *
 * CLI
 * ------------------------------------------------------------------ */

function parseArgs(argv) {
  const o = {
    shuttle: null,
    grammar: 'turtle12',
    seed: 7,
    count: 400,
    mutants: 400,
    out: null,
    compare: [],
    report: null,
    selfTest: false,
    maxTokens: 90,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--shuttle') o.shuttle = argv[++i];
    else if (a === '--grammar') o.grammar = argv[++i];
    else if (a === '--seed') o.seed = Number(argv[++i]);
    else if (a === '--count') o.count = Number(argv[++i]);
    else if (a === '--mutants') o.mutants = Number(argv[++i]);
    else if (a === '--max-tokens') o.maxTokens = Number(argv[++i]);
    else if (a === '--out') o.out = argv[++i];
    else if (a === '--compare') o.compare.push(argv[++i]);
    else if (a === '--report') o.report = argv[++i];
    else if (a === '--self-test') o.selfTest = true;
    else {
      console.error(`unknown argument: ${a}`);
      process.exit(2);
    }
  }
  if (!o.shuttle) {
    console.error('usage: shuttle-generate-harvest.mjs --shuttle <rdf-shuttle checkout> [...]');
    process.exit(2);
  }
  return o;
}

/* ------------------------------------------------------------------ *
 * Seeded PRNG (mulberry32) — every run is reproducible from --seed.
 * ------------------------------------------------------------------ */

function rng(seed) {
  let s = seed >>> 0;
  return function next() {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/* ------------------------------------------------------------------ *
 * Terminal sampling pools.
 *
 * BOUNDARY-BIASED by hand: escape classes, PN_LOCAL dot/colon/percent
 * hazards, long-string quote runs, directional language tags, numeric
 * sign/exponent corners. The real generate mode derives these from the
 * token patterns; this table is the interim's largest hand-written
 * surface and the honest limit on its coverage claim.
 *
 * Prefixes are restricted to the ones the generated preamble declares, so
 * a derived document does not trip UNDECLARED_PREFIX by accident (the
 * negative pass reaches that code deliberately).
 * ------------------------------------------------------------------ */

const POOLS = {
  // keyword tokens: the case-insensitive SPARQL-style forms are sampled too
  AT_PREFIX: ['@prefix'],
  AT_BASE: ['@base'],
  AT_VERSION: ['@version'],
  KW_PREFIX: ['PREFIX', 'prefix', 'PreFiX'],
  KW_BASE: ['BASE', 'base', 'BaSe'],
  KW_VERSION: ['VERSION', 'version'],
  IRIREF: [
    '<http://example.org/a>',
    '<http://example.org/b>',
    '<>',
    '<#frag>',
    '<rel/x>',
    '<http://example.org/p%C3%A9>',
    '<http://example.org/\\u00E9>',
    '<http://a.example/s?q=1&r=2#f>',
    '<http://example.org/\\U0001F600x>',
  ],
  PNAME_NS: [':', 'ex:', 'xsd:'],
  PNAME_LN: [
    'ex:a',
    ':b',
    'ex:a.b',
    'ex:a:b',
    'ex:0',
    'ex:_',
    'ex:x%20y',
    'ex:a\\-b',
    'ex:a\\.b',
    ':\\~t',
  ],
  BLANK_NODE_LABEL: ['_:b0', '_:b1', '_:x', '_:0', '_:a.b', '_:a-b'],
  ANON: ['[]', '[ ]', '[\n]'],
  LANG_DIR: ['@en', '@en-GB', '@EN', '@ar--rtl', '@en--ltr', '@x-y-z'],
  INTEGER: ['0', '7', '-3', '+12', '007'],
  DECIMAL: ['0.5', '-0.0', '+3.14', '.5', '10.00'],
  DOUBLE: ['1e0', '-2.5E10', '.5e-3', '1.0e+2', '0E0'],
  STRING_LITERAL_QUOTE: [
    '"x"',
    '""',
    '"a\\"b"',
    '"\\u00E9"',
    '"tab\\there"',
    '"\\U0001F600"',
    '"a\\\\b"',
    '"line\\nbreak"',
    '"\'"',
  ],
  STRING_LITERAL_SINGLE_QUOTE: ["'x'", "''", "'a\"b'", "'\\u00E9'", "'a\\'b'"],
  STRING_LITERAL_LONG_QUOTE: ['"""x"""', '"""a\nb"""', '"""he said ""hi"" ok"""', '"""q"end"""'],
  STRING_LITERAL_LONG_SINGLE_QUOTE: ["'''x'''", "'''a\nb'''", "'''it''s ok'''"],
};

// `versionSpecifier` reads a string token but the grammar constrains nothing;
// RDF 1.2 only defines "1.2". Keep the corpus mostly legal so a VERSION
// disagreement is a signal rather than noise.
const VERSION_POOL = ['"1.2"', '"1.2"', '"1.2"', "'1.2'", '"1.1"'];

const PREAMBLE = [
  '@prefix ex: <http://example.org/> .',
  '@prefix : <http://example.org/d#> .',
  '@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .',
  '',
].join('\n');

const BASE_IRI = 'http://example.org/doc';

const SEPARATORS = [' ', ' ', ' ', '\n', '\t', ' #c\n', '\n\n'];

/* ------------------------------------------------------------------ *
 * Grammar loading + minimal-expansion cost (the depth-cap escape hatch)
 * ------------------------------------------------------------------ */

async function loadGrammar(shuttleDir, grammarName) {
  const metaUrl = pathToFileURL(path.join(shuttleDir, 'packages/gen-js/src/meta.js'));
  const { parseGrammar } = await import(metaUrl.href);
  const grammarFile = path.join(shuttleDir, 'grammars', `${grammarName}.shuttle`);
  const g = parseGrammar(fs.readFileSync(grammarFile, 'utf8'), grammarFile);
  const parserUrl = pathToFileURL(path.join(shuttleDir, 'packages/gen-js/generated', `${grammarName}.js`));
  const parser = await import(parserUrl.href);
  const isoUrl = pathToFileURL(path.join(shuttleDir, 'packages/gen-js/test/iso.js'));
  const iso = await import(isoUrl.href);
  return { g, parser, iso, grammarFile };
}

/** Least number of terminals an alternative/production can expand to. */
function computeCosts(g) {
  const prodCost = new Map();
  for (const p of g.prods) prodCost.set(p.name, Infinity);
  const itemCost = (item) => {
    if (item.kind === 'sem') return 0;
    if (item.kind === 'thread') return item.rep === 'plus' ? altsCost(item.body) : 0;
    const base =
      item.prim.kind === 'lit' || item.prim.kind === 'token'
        ? 1
        : item.prim.kind === 'group'
          ? altsCost(item.prim.alts)
          : (prodCost.get(item.prim.name) ?? Infinity);
    if (item.postfix === 'opt' || item.postfix === 'star') return 0;
    return base;
  };
  const altCost = (alt) => alt.items.reduce((n, it) => n + itemCost(it), 0);
  const altsCost = (alts) => Math.min(...alts.map(altCost));
  for (let round = 0; round < g.prods.length + 2; round++) {
    let changed = false;
    for (const p of g.prods) {
      const c = altsCost(p.alts);
      if (c < prodCost.get(p.name)) {
        prodCost.set(p.name, c);
        changed = true;
      }
    }
    if (!changed) break;
  }
  return { prodCost, altCost, altsCost, itemCost };
}

/* ------------------------------------------------------------------ *
 * Coverage-directed derivation
 * ------------------------------------------------------------------ */

class Deriver {
  constructor(g, costs, rand, opts) {
    this.g = g;
    this.costs = costs;
    this.rand = rand;
    this.maxTokens = opts.maxTokens;
    this.coverage = opts.coverage; // Map key -> count, shared across documents
    this.covers = opts.covers; // Map @covers label -> count
    this.ctx = []; // enclosing production names (context-sensitive sampling)
  }

  /** Pick the least-covered alternative, breaking ties with the PRNG. */
  pick(key, alts, budget) {
    const affordable = [];
    for (let i = 0; i < alts.length; i++) {
      const c = this.costs.altCost(alts[i]);
      if (c <= budget) affordable.push(i);
    }
    const pool = affordable.length ? affordable : [alts.map((a) => this.costs.altCost(a)).reduce((best, c, i, arr) => (c < arr[best] ? i : best), 0)];
    let bestN = Infinity;
    let best = [];
    for (const i of pool) {
      const n = this.coverage.get(`${key}#${i}`) ?? 0;
      if (n < bestN) {
        bestN = n;
        best = [i];
      } else if (n === bestN) best.push(i);
    }
    const idx = best[Math.floor(this.rand() * best.length)];
    this.coverage.set(`${key}#${idx}`, (this.coverage.get(`${key}#${idx}`) ?? 0) + 1);
    const cov = alts[idx].annots && alts[idx].annots.covers;
    if (cov) this.covers.set(cov, (this.covers.get(cov) ?? 0) + 1);
    return idx;
  }

  sampleToken(name) {
    // Context-sensitive: inside `versionSpecifier` the token is a string token,
    // but RDF 1.2 defines exactly one legal value. Sampling arbitrary strings
    // there would drown every other signal in one known grammar-vs-spec gap, so
    // the pool is mostly "1.2" and reaches the gap deliberately, rarely.
    const pool = this.ctx.includes('versionSpecifier') && String(name).startsWith('STRING_LITERAL')
      ? VERSION_POOL
      : POOLS[name];
    if (!pool) return null; // fragment / unknown token: caller falls back
    return pool[Math.floor(this.rand() * pool.length)];
  }

  emitAlt(key, alts, out, depth, budget) {
    const idx = this.pick(key, alts, budget);
    this.emitItems(`${key}#${idx}`, alts[idx].items, out, depth, budget);
  }

  emitItems(key, items, out, depth, budget) {
    for (let i = 0; i < items.length; i++) {
      this.emitItem(`${key}.${i}`, items[i], out, depth, budget);
    }
  }

  reps(postfix, budget, unitCost) {
    if (postfix === 'opt') return this.rand() < 0.5 && unitCost <= budget ? 1 : 0;
    const room = unitCost > 0 ? Math.floor(budget / unitCost) : 3;
    const want = postfix === 'plus' ? 1 + Math.floor(this.rand() * 2) : Math.floor(this.rand() * 3);
    return Math.max(postfix === 'plus' ? 1 : 0, Math.min(want, Math.max(room, postfix === 'plus' ? 1 : 0)));
  }

  expandProd(p, out, depth, budget, room) {
    if (depth > 12 || room <= 0) {
      // Depth/size cap: take the cheapest closing alternative rather than
      // truncating mid-derivation (a truncated document is not a document).
      const costsArr = p.alts.map((a) => this.costs.altCost(a));
      const idx = costsArr.indexOf(Math.min(...costsArr));
      this.coverage.set(`${p.name}#${idx}`, (this.coverage.get(`${p.name}#${idx}`) ?? 0) + 1);
      this.emitItems(`${p.name}#${idx}`, p.alts[idx].items, out, depth + 1, 1);
      return;
    }
    this.emitAlt(p.name, p.alts, out, depth + 1, Math.min(budget, room));
  }

  emitItem(key, item, out, depth, budget) {
    if (item.kind === 'sem') return;
    if (item.kind === 'thread') {
      const n = this.reps(item.rep === 'plus' ? 'plus' : 'star', budget, Math.max(1, this.costs.altsCost(item.body)));
      for (let i = 0; i < n; i++) this.emitAlt(`${key}~thread`, item.body, out, depth + 1, budget);
      return;
    }
    const unit = () => {
      if (item.prim.kind === 'lit') {
        out.push(item.prim.text);
        return;
      }
      if (item.prim.kind === 'token') {
        const lex = this.sampleToken(item.prim.name);
        out.push(lex === null ? item.prim.name : lex);
        return;
      }
      if (item.prim.kind === 'group') {
        this.emitAlt(`${key}(g)`, item.prim.alts, out, depth + 1, budget);
        return;
      }
      const p = this.g.prodByName.get(item.prim.name);
      if (!p) throw new Error(`unknown production ${item.prim.name}`);
      const room = this.maxTokens - out.length;
      this.ctx.push(p.name);
      try {
        this.expandProd(p, out, depth, budget, room);
      } finally {
        this.ctx.pop();
      }
    };
    if (item.postfix === 'sepList') {
      const n = 1 + (this.rand() < 0.4 ? 1 : 0);
      for (let i = 0; i < n; i++) {
        if (i) out.push(item.sep);
        unit();
      }
      return;
    }
    const n = item.postfix ? this.reps(item.postfix, this.maxTokens - out.length, 1) : 1;
    for (let i = 0; i < n; i++) unit();
  }

  document() {
    const out = [];
    const start = this.g.headers.start || this.g.prods[0].name;
    const p = this.g.prodByName.get(start);
    // turtleDoc ::= statement* — drive the statement count directly so a
    // document is never empty and stays small enough to triage by hand.
    const stmts = 1 + Math.floor(this.rand() * 3);
    const stmt = this.g.prodByName.get('statement') ?? p;
    // The document IS the start production's `statement*` alternative; record it
    // so the coverage denominator is not silently short by one.
    this.coverage.set(`${start}#0`, (this.coverage.get(`${start}#0`) ?? 0) + 1);
    for (let i = 0; i < stmts; i++) {
      if (out.length >= this.maxTokens) break;
      this.emitAlt(stmt.name, stmt.alts, out, 1, this.maxTokens - out.length);
    }
    return out;
  }
}

/** Join a token list with sampled whitespace (always non-empty: safe by construction). */
function render(tokens, rand) {
  let s = '';
  for (let i = 0; i < tokens.length; i++) {
    if (i) s += SEPARATORS[Math.floor(rand() * SEPARATORS.length)];
    s += tokens[i];
  }
  return `${s}\n`;
}

/* ------------------------------------------------------------------ *
 * Oracle + canonical rendering
 * ------------------------------------------------------------------ */

function termToNt(t) {
  switch (t.termType) {
    case 'NamedNode':
      return `<${t.value}>`;
    case 'BlankNode':
      return `_:${t.value}`;
    case 'Literal': {
      const lex = JSON.stringify(t.value);
      if (t.language) return `${lex}@${t.language}${t.direction ? `--${t.direction}` : ''}`;
      return `${lex}^^<${t.datatype.value}>`;
    }
    case 'Quad':
      return `<<( ${termToNt(t.subject)} ${termToNt(t.predicate)} ${termToNt(t.object)} )>>`;
    case 'DefaultGraph':
      return '';
    default:
      throw new Error(`termToNt: ${t.termType}`);
  }
}

function quadsToNt(quads) {
  return quads
    .map((q) => `${termToNt(q.subject)} ${termToNt(q.predicate)} ${termToNt(q.object)} .`)
    .sort()
    .join('\n');
}

/* ------------------------------------------------------------------ *
 * Comparators (the parsers under differential test)
 * ------------------------------------------------------------------ */

/**
 * oxigraph:<path to the npm package's node.js> — Oxigraph's Turtle parser IS oxttl,
 * so this measures oxttl too.
 *
 * MUST use `parse()`, not `Store.load()` + `match()`: the store CANONICALIZES
 * numeric literal lexical forms (`"1.0e+2"^^xsd:double` comes back as `"100"`,
 * `"-0.0"^^xsd:decimal` as `"0"`), which is a store design choice, not a parser
 * behaviour. Comparing through the store reported those as parser divergences —
 * a harness bug that produced ~50 false positives before this was fixed.
 */
async function oxigraphComparator(modPath) {
  const ox = await import(pathToFileURL(path.resolve(modPath)).href);
  const mod = ox.default ?? ox;
  return {
    name: `oxigraph ${modPath}`,
    parse(text) {
      return mod.parse(text, { format: 'text/turtle', base_iri: BASE_IRI });
    },
  };
}

/** Seeded mutants — the non-vacuity proof for --self-test, never a real target. */
function mutantComparator(kind, oracleParse) {
  return {
    name: `MUTANT:${kind}`,
    parse(text) {
      if (kind === 'accept-all') {
        try {
          return oracleParse(text);
        } catch {
          return []; // accepts what the oracle rejects
        }
      }
      const quads = oracleParse(text);
      if (kind === 'drop-quad') return quads.slice(1);
      throw new Error(`unknown mutant ${kind}`);
    },
  };
}

async function buildComparators(specs, oracleParse) {
  const out = [];
  for (const spec of specs) {
    const i = spec.indexOf(':');
    const kind = i < 0 ? spec : spec.slice(0, i);
    const arg = i < 0 ? '' : spec.slice(i + 1);
    if (kind === 'oxigraph') out.push(await oxigraphComparator(arg));
    else if (kind === 'mutant') out.push(mutantComparator(arg, oracleParse));
    else throw new Error(`unknown --compare kind: ${kind}`);
  }
  return out;
}

/* ------------------------------------------------------------------ *
 * Divergence detection + statement-level shrink
 * ------------------------------------------------------------------ */

/**
 * TOKEN-level ddmin-lite: greedily drop tokens while the document still parses on
 * the ORACLE and still diverges on the candidate. Token-level (not line-level as
 * in the Rust harness) because a derived Turtle document is one statement with
 * whitespace sampled INSIDE it — dropping a line is almost never a legal edit,
 * so a line shrink returns the whole document and the repro is unreadable.
 */
function shrink(tokens, stillDiverges) {
  let cur = tokens.slice();
  // Chunk sizes descend from half the document to one token. Single-token
  // removal alone barely shrinks a Turtle document — almost every one-token edit
  // makes it unparseable, so the greedy pass stalls at the first token and the
  // "minimal repro" comes back the size of the original.
  for (let size = Math.max(1, Math.floor(cur.length / 2)); size >= 1; ) {
    let removed = false;
    for (let i = 0; i + size <= cur.length; i++) {
      const trial = cur.slice(0, i).concat(cur.slice(i + size));
      if (trial.length && stillDiverges(trial)) {
        cur = trial;
        removed = true;
        break;
      }
    }
    if (!removed) size = Math.floor(size / 2);
    else if (size > cur.length) size = cur.length;
  }
  return cur;
}

/** Canonical single-space rendering — the form a repro is quoted in. */
function renderPlain(tokens) {
  return `${tokens.join(' ')}\n`;
}

function comparePositive(cand, doc, expectedQuads, iso) {
  let got;
  try {
    got = cand.parse(PREAMBLE + doc);
  } catch (e) {
    return { kind: 'candidate-rejects', detail: String(e && e.message ? e.message : e) };
  }
  if (!iso.isIsomorphic(got, expectedQuads)) {
    return {
      kind: 'quad-set',
      detail: `expected ${expectedQuads.length} quad(s), candidate produced ${got.length}`,
      expected: quadsToNt(expectedQuads),
      actual: quadsToNt(got),
    };
  }
  return null;
}

function compareNegative(cand, doc) {
  try {
    cand.parse(PREAMBLE + doc);
  } catch {
    return null; // rejected, as required
  }
  return { kind: 'candidate-accepts', detail: 'oracle rejects this document; candidate accepts it' };
}

/* ------------------------------------------------------------------ *
 * Mutation (the interim negative family)
 * ------------------------------------------------------------------ */

function mutate(tokens, rand) {
  const t = tokens.slice();
  if (!t.length) return t;
  const op = Math.floor(rand() * 4);
  const i = Math.floor(rand() * t.length);
  if (op === 0) t.splice(i, 1); // delete a token
  else if (op === 1) t.splice(i, 0, t[i]); // duplicate a token
  else if (op === 2 && t.length > 1) {
    // swap adjacent tokens
    const j = Math.min(i + 1, t.length - 1);
    [t[i], t[j]] = [t[j], t[i]];
  } else {
    // substitute an undeclared prefix (reaches the grammar's UNDECLARED_PREFIX
    // `require` clause) or a stray delimiter
    t[i] = rand() < 0.5 ? 'nope:x' : ['<<', '{|', ')>>', ';'][Math.floor(rand() * 4)];
  }
  return t;
}

/* ------------------------------------------------------------------ *
 * Corpus writing (layout the sq-tonhr.2 Rust harness can walk)
 * ------------------------------------------------------------------ */

/**
 * The upstream commit the corpus was generated from — a bare `.git/HEAD` read is
 * not enough, since a normal clone leaves it a symref (`ref: refs/heads/main`),
 * and a corpus header that records a branch name instead of a commit pins nothing.
 */
function readGitHead(repoDir) {
  const git = path.join(repoDir, '.git');
  let head;
  try {
    head = fs.readFileSync(path.join(git, 'HEAD'), 'utf8').trim();
  } catch {
    return 'unknown';
  }
  const sym = /^ref:\s*(\S+)$/.exec(head);
  if (!sym) return head; // already a detached-HEAD sha
  try {
    return fs.readFileSync(path.join(git, sym[1]), 'utf8').trim();
  } catch {
    // packed refs: `<sha> <refname>` per line
    try {
      const packed = fs.readFileSync(path.join(git, 'packed-refs'), 'utf8');
      for (const line of packed.split('\n')) {
        const m = /^([0-9a-f]{40})\s+(\S+)$/.exec(line);
        if (m && m[2] === sym[1]) return m[1];
      }
    } catch {
      /* fall through */
    }
    return `unresolved ${sym[1]}`;
  }
}

function writeCorpus(dir, positives, negatives, meta) {
  fs.mkdirSync(path.join(dir, 'positive'), { recursive: true });
  fs.mkdirSync(path.join(dir, 'negative'), { recursive: true });
  const entries = [];
  positives.forEach((p, i) => {
    const stem = `gen-eval-${String(i + 1).padStart(4, '0')}`;
    fs.writeFileSync(path.join(dir, 'positive', `${stem}.ttl`), PREAMBLE + p.doc);
    fs.writeFileSync(path.join(dir, 'positive', `${stem}.nt`), `${p.expectedNt}\n`);
    entries.push({ stem, type: 'rdft:TestTurtleEval', dir: 'positive' });
  });
  negatives.forEach((n, i) => {
    const stem = `gen-neg-${String(i + 1).padStart(4, '0')}`;
    fs.writeFileSync(path.join(dir, 'negative', `${stem}.ttl`), PREAMBLE + n.doc);
    entries.push({ stem, type: 'rdft:TestTurtleNegativeSyntax', dir: 'negative', code: n.code });
  });
  const ttl = [
    '# GENERATED by scripts/shuttle-generate-harvest.mjs — do not edit by hand.',
    `# grammar: ${meta.grammar}   shuttle pin: ${meta.pin}   seed: ${meta.seed}`,
    '@prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .',
    '@prefix rdft: <http://www.w3.org/ns/rdftest#> .',
    '@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .',
    '',
    '<> a mf:Manifest ;',
    `  rdfs:label "Shuttle interim generate-mode corpus (${meta.grammar}, seed ${meta.seed})" ;`,
    '  mf:entries (',
    ...entries.map((e) => `    <#${e.stem}>`),
    '  ) .',
    '',
    ...entries.flatMap((e) => [
      `<#${e.stem}> a ${e.type} ;`,
      `  mf:name "${e.stem}" ;`,
      `  mf:action <${e.dir}/${e.stem}.ttl> ;`,
      ...(e.type === 'rdft:TestTurtleEval' ? [`  mf:result <${e.dir}/${e.stem}.nt> ;`] : []),
      ...(e.code ? [`  rdfs:comment "oracle error code: ${e.code}" ;`] : []),
      '  .',
      '',
    ]),
  ].join('\n');
  fs.writeFileSync(path.join(dir, 'manifest.ttl'), ttl);
}

/* ------------------------------------------------------------------ *
 * Main
 * ------------------------------------------------------------------ */

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const { g, parser, iso, grammarFile } = await loadGrammar(opts.shuttle, opts.grammar);
  const costs = computeCosts(g);
  const rand = rng(opts.seed);
  const coverage = new Map();
  const covers = new Map();
  const deriver = new Deriver(g, costs, rand, { maxTokens: opts.maxTokens, coverage, covers });

  const oracleParse = (text) => parser.parseToQuads(text, { baseIRI: BASE_IRI }).quads;

  // --- pass 1: coverage-directed positives, filtered by the oracle ----------
  const positives = [];
  let derived = 0;
  let oracleRejected = 0;
  const rejectCodes = new Map();
  while (positives.length < opts.count && derived < opts.count * 12) {
    derived++;
    let tokens;
    try {
      tokens = deriver.document();
    } catch {
      continue;
    }
    const doc = render(tokens, rand);
    let quads;
    try {
      quads = oracleParse(PREAMBLE + doc);
    } catch (e) {
      oracleRejected++;
      const code = (e && e.code) || 'UNKNOWN';
      rejectCodes.set(code, (rejectCodes.get(code) ?? 0) + 1);
      continue;
    }
    positives.push({ tokens, doc, quads, expectedNt: quadsToNt(quads) });
  }

  // --- pass 2: oracle-rejected mutants (the interim negative family) --------
  const negatives = [];
  const codeBuckets = new Map();
  const seenNeg = new Set();
  for (let i = 0; i < opts.mutants && positives.length; i++) {
    const src = positives[Math.floor(rand() * positives.length)];
    const doc = render(mutate(src.tokens, rand), rand);
    if (seenNeg.has(doc)) continue;
    try {
      oracleParse(PREAMBLE + doc);
      continue; // still legal: not a negative test
    } catch (e) {
      const code = (e && e.code) || 'UNKNOWN';
      seenNeg.add(doc);
      negatives.push({ doc, code });
      codeBuckets.set(code, (codeBuckets.get(code) ?? 0) + 1);
    }
  }

  // --- pass 3: harvest — run every corpus document through each comparator --
  const compareSpecs = opts.selfTest ? ['mutant:drop-quad', 'mutant:accept-all'] : opts.compare;
  const comparators = await buildComparators(compareSpecs, oracleParse);
  const findings = [];
  for (const cand of comparators) {
    const divergences = [];
    for (const p of positives) {
      const d = comparePositive(cand, p.doc, p.quads, iso);
      if (!d) continue;
      const stillDiverges = (toks) => {
        const doc = renderPlain(toks);
        let expected;
        try {
          expected = oracleParse(PREAMBLE + doc);
        } catch {
          return false; // no longer a positive: not the same divergence
        }
        const d2 = comparePositive(cand, doc, expected, iso);
        return d2 !== null && d2.kind === d.kind;
      };
      const repro = stillDiverges(p.tokens) ? renderPlain(shrink(p.tokens, stillDiverges)) : p.doc;
      divergences.push({ corpus: 'positive', ...d, repro: repro.trim() });
    }
    for (const n of negatives) {
      const d = compareNegative(cand, n.doc);
      if (!d) continue;
      divergences.push({ corpus: 'negative', code: n.code, ...d, repro: n.doc.trim() });
    }
    findings.push({ candidate: cand.name, divergences });
  }

  // --- report ---------------------------------------------------------------
  // Coverage denominator: one entry per TOP-LEVEL (production, alternative).
  // Nested group/thread alternatives are counted separately by `coverage` keys
  // but deliberately excluded here — the denominator has to be a number the
  // grammar file itself states, or the ratio is unfalsifiable.
  const altTotal = g.prods.reduce((n, p) => n + p.alts.length, 0);
  const topLevelKey = /^([A-Za-z_][A-Za-z0-9_]*)#(\d+)$/;
  const hit = new Set();
  const missing = [];
  for (const k of coverage.keys()) {
    const m = topLevelKey.exec(k);
    if (m && g.prodByName.has(m[1])) hit.add(k);
  }
  for (const p of g.prods) {
    for (let i = 0; i < p.alts.length; i++) if (!hit.has(`${p.name}#${i}`)) missing.push(`${p.name}#${i}`);
  }
  const altHit = hit.size;
  const report = {
    grammar: opts.grammar,
    grammarFile,
    seed: opts.seed,
    documentsDerived: derived,
    positivesKept: positives.length,
    oracleRejectedDuringDerivation: oracleRejected,
    oracleRejectCodes: Object.fromEntries(rejectCodes),
    negativesKept: negatives.length,
    negativeErrorCodes: Object.fromEntries(codeBuckets),
    coversLabels: Object.fromEntries(covers),
    topLevelAlternativesCovered: `${altHit}/${altTotal}`,
    topLevelAlternativesMissed: missing,
    candidates: findings.map((f) => ({
      candidate: f.candidate,
      divergences: f.divergences.length,
      byKind: f.divergences.reduce((m, d) => ({ ...m, [d.kind]: (m[d.kind] ?? 0) + 1 }), {}),
    })),
    divergences: findings,
  };
  if (opts.report) fs.writeFileSync(opts.report, `${JSON.stringify(report, null, 2)}\n`);
  if (opts.out) {
    const pin = readGitHead(opts.shuttle);
    writeCorpus(opts.out, positives, negatives, { grammar: opts.grammar, seed: opts.seed, pin });
  }

  const summary = {
    ...report,
    divergences: undefined,
  };
  console.log(JSON.stringify(summary, null, 2));

  if (opts.selfTest) {
    // NON-VACUITY: each seeded mutant MUST be caught. If either reports zero
    // divergences the comparison path is broken and this exits non-zero.
    const failures = findings.filter((f) => f.divergences.length === 0).map((f) => f.candidate);
    if (positives.length === 0) {
      console.error('self-test FAILED: derivation produced no oracle-accepted documents');
      process.exit(1);
    }
    if (failures.length) {
      console.error(`self-test FAILED: seeded mutants not detected: ${failures.join(', ')}`);
      process.exit(1);
    }
    console.error(`self-test OK: ${findings.map((f) => `${f.candidate}=${f.divergences.length}`).join(' ')}`);
  }
}

main().catch((e) => {
  console.error(e && e.stack ? e.stack : String(e));
  process.exit(1);
});
