// [FABLE-5] sq-ixc3.20 — pure view logic for the why() proof-tree panel (no React, no DOM),
// unit-tested with the gui/app node test runner. The React panel
// (components/workbench/proof-panel.tsx) is a thin renderer over these.
//
// The wire shape is `sparq-reason`'s `ProofTree::to_json` (the sq-6qw3 contract): a FLAT,
// premises-before-conclusion DAG —
//   `{"root":R,"nodes":[{"id":i,"conclusion":[s,p,o],"rule":"…","premises":[…]},…]}`
// where every premise index is strictly LESS than its node's own id, the root is the last
// node, leaves are `"asserted"` base facts (or premise-free `axiom-*` tautologies), and a
// shared sub-derivation appears ONCE but may be referenced by several `premises` lists. The
// display shaping below expands that DAG into a render tree, marking REPEAT visits so the
// panel can show "see step #n" instead of re-rendering (and so the expansion is linear, not
// exponential, on heavily-shared proofs).

import { abbreviateIri } from "./iri-label.js";

// ---------------------------------------------------------------------------
// The wire schema + defensive parse (mirrors @sparq/client's parsePlanJson idiom).
// ---------------------------------------------------------------------------

/** One node of the proof DAG (`sparq-reason` `ProofNode` + its array index as `id`). */
export interface ProofNodeJson {
  id: number;
  /** Three self-contained N-Triples/N3 term strings: subject, predicate, object. */
  conclusion: [string, string, string];
  /** `"asserted"` (leaf), an `axiom-*` tautology, a W3C rule id (`rdfs9`, `cax-sco`, …),
   *  or `n3-rule-<i>` for a user N3 rule. */
  rule: string;
  /** Indices of premise nodes — each strictly less than {@link id}. */
  premises: number[];
}

/** The whole proof: a flat premises-before-conclusion DAG; `root` is the explained triple. */
export interface ProofTreeJson {
  root: number;
  nodes: ProofNodeJson[];
}

function asIndex(v: unknown, field: string, max: number): number {
  if (typeof v !== "number" || !Number.isInteger(v) || v < 0 || v >= max) {
    throw new Error(`proof field "${field}" is not a valid node index`);
  }
  return v;
}

/**
 * Validate one parsed JSON value as a {@link ProofTreeJson} (throws on shape drift). The
 * premises-strictly-before-conclusion invariant is ENFORCED, not assumed — it is what makes
 * the display expansion below provably terminate.
 */
export function parseProofTree(value: unknown): ProofTreeJson {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("proof tree is not an object");
  }
  const o = value as Record<string, unknown>;
  if (!Array.isArray(o.nodes) || o.nodes.length === 0) {
    throw new Error('proof tree lacks a non-empty "nodes" array');
  }
  const nodes = o.nodes.map((raw, i): ProofNodeJson => {
    if (typeof raw !== "object" || raw === null) throw new Error(`proof node ${i} is not an object`);
    const n = raw as Record<string, unknown>;
    const id = asIndex(n.id, "id", (o.nodes as unknown[]).length);
    if (id !== i) throw new Error(`proof node ${i} has out-of-place id ${id}`);
    if (
      !Array.isArray(n.conclusion) ||
      n.conclusion.length !== 3 ||
      n.conclusion.some((t) => typeof t !== "string")
    ) {
      throw new Error(`proof node ${i} lacks a 3-term "conclusion"`);
    }
    if (typeof n.rule !== "string" || n.rule === "") {
      throw new Error(`proof node ${i} lacks a "rule" label`);
    }
    const premises = n.premises === undefined ? [] : n.premises;
    if (!Array.isArray(premises)) throw new Error(`proof node ${i} "premises" is not an array`);
    for (const p of premises) {
      // Strictly-before (`premise < id`): the invariant that guarantees the display
      // expansion is finite and acyclic.
      asIndex(p, `node ${i} premises[]`, i);
    }
    return {
      id,
      conclusion: [n.conclusion[0], n.conclusion[1], n.conclusion[2]] as [string, string, string],
      rule: n.rule,
      premises: premises as number[],
    };
  });
  const root = asIndex(o.root, "root", nodes.length);
  return { root, nodes };
}

/**
 * Parse a `why()` JSON *string* (what the wasm binding returns): the proof tree, or `null`
 * for the JSON literal `null` (the triple is not entailed — nothing to explain). Malformed
 * JSON throws (surfaced honestly by the panel, never rendered as an empty proof).
 */
export function parseProofJson(json: string): ProofTreeJson | null {
  const value: unknown = JSON.parse(json);
  if (value === null) return null;
  return parseProofTree(value);
}

// ---------------------------------------------------------------------------
// Rule classification (drives the honest asserted-vs-derived labelling).
// ---------------------------------------------------------------------------

/** How a proof node's conclusion came to hold. */
export type ProofRuleKind = "asserted" | "axiom" | "rule";

/** Classify a node's `rule` label: an asserted base fact, a premise-free axiomatic
 *  tautology, or a real rule firing (RDFS/OWL-RL rule id or `n3-rule-<i>`). */
export function ruleKind(rule: string): ProofRuleKind {
  if (rule === "asserted") return "asserted";
  if (rule.startsWith("axiom")) return "axiom";
  return "rule";
}

/** A short human label for an N3 rule id (`n3-rule-2` → `rule #3`); other ids verbatim. */
export function ruleLabel(rule: string): string {
  const m = /^n3-rule-(\d+)$/.exec(rule);
  if (m) return `N3 rule #${Number.parseInt(m[1], 10) + 1}`;
  return rule;
}

// ---------------------------------------------------------------------------
// Display shaping: DAG → render tree with repeat markers.
// ---------------------------------------------------------------------------

/** One row of the rendered proof tree. */
export interface ProofDisplayNode {
  /** The underlying DAG node id (`#n` in the panel — stable across repeats). */
  id: number;
  rule: string;
  kind: ProofRuleKind;
  /** The three N-Triples/N3 term strings of the concluded triple. */
  conclusion: [string, string, string];
  /** True when this DAG node was already expanded elsewhere in the tree: render as a
   *  "see step #n" reference and do NOT re-render its children. */
  repeat: boolean;
  children: ProofDisplayNode[];
}

/**
 * Expand the flat DAG into a render tree rooted at `tree.root`. A node SHARED by several
 * premises lists is fully expanded on its FIRST (depth-first) visit and becomes a `repeat`
 * reference on every later one — linear in the DAG size by construction.
 */
export function buildProofDisplay(tree: ProofTreeJson): ProofDisplayNode {
  const expanded = new Set<number>();
  const build = (id: number): ProofDisplayNode => {
    const n = tree.nodes[id];
    const repeat = expanded.has(id);
    if (!repeat) expanded.add(id);
    return {
      id,
      rule: n.rule,
      kind: ruleKind(n.rule),
      conclusion: n.conclusion,
      repeat,
      children: repeat ? [] : n.premises.map(build),
    };
  };
  return build(tree.root);
}

/** The one-line proof summary the panel header shows (all derived, none fabricated). */
export interface ProofSummary {
  /** Distinct derivation steps (DAG nodes). */
  steps: number;
  /** Nodes that are real rule firings (excludes asserted leaves + axioms). */
  ruleFirings: number;
  /** Asserted base-fact leaves the proof bottoms out in. */
  assertedLeaves: number;
}

/** Derive the {@link ProofSummary} of a proof DAG. */
export function proofSummary(tree: ProofTreeJson): ProofSummary {
  let ruleFirings = 0;
  let assertedLeaves = 0;
  for (const n of tree.nodes) {
    const k = ruleKind(n.rule);
    if (k === "rule") ruleFirings++;
    else if (k === "asserted") assertedLeaves++;
  }
  return { steps: tree.nodes.length, ruleFirings, assertedLeaves };
}

// ---------------------------------------------------------------------------
// Term display (proof conclusions are self-contained N-Triples/N3 term strings).
// ---------------------------------------------------------------------------

/**
 * A short, human caption for one proof-term string (`<iri>` → prefixed/last-segment form,
 * `"lit"^^<dt>` → the shortened quoted form, `_:b` verbatim). The FULL term string stays
 * available to the panel (shown on the row's detail toggle) — this is display, not identity.
 */
export function ntTermLabel(term: string): string {
  if (term.startsWith("<") && term.endsWith(">")) {
    return abbreviateIri(term.slice(1, -1));
  }
  if (term.startsWith('"')) {
    // Find the closing quote of the lexical form (escape-aware), drop any @lang/^^dt tail.
    let end = -1;
    for (let i = 1; i < term.length; i++) {
      if (term[i] === "\\") i++;
      else if (term[i] === '"') {
        end = i;
        break;
      }
    }
    const lex = end > 0 ? term.slice(1, end) : term.slice(1);
    return lex.length > 24 ? `"${lex.slice(0, 23)}…"` : `"${lex}"`;
  }
  return term; // blank node / anything else: verbatim
}
