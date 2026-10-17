/**
 * OWL 2 DL reasoning: consistency, the OWL 2 DL profile check, unsatisfiable classes and the
 * inferred class hierarchy, from the `sparq-reason-dl` hypertableau reasoner compiled to its
 * OWN wasm bundle (`wasm-dl/`, built from `crates/sparq-dl-wasm`).
 *
 * The bundle is loaded LAZILY: nothing here imports it statically, so a page that imports
 * `@sparq-org/sparq` but never calls {@link checkOwlDl} never fetches it. The first call
 * imports `../wasm-dl/sparq_dl_wasm.js` dynamically and initialises it (in Node from the bytes
 * on disk, elsewhere by `fetch` relative to the module); the load is memoised and a failed
 * initialisation is retried on the next call (a browser keeps a failed glue-module fetch cached).
 *
 * Every answer is three-valued and fail-closed: when the reasoner gives no verdict the result
 * says `"unknown"` with the reason and, where there are any, the offending input triples.
 */

/** An input triple, rendered as one N-Triples line (`<s> <p> <o> .`). */
export type OwlDlTriple = string;

/** The kind of an OWL 2 DL profile violation (OWL 2 Structural Specification §5.8.1, §11, §4, §9.4). */
export type OwlDlViolationKind =
  | 'Undeclared'
  | 'IllegalPunning'
  | 'ReservedVocabulary'
  | 'NonSimpleProperty'
  | 'IrregularPropertyHierarchy'
  | 'AnonymousIndividualCycle'
  | 'IllTypedLiteral'
  | 'UnknownDatatype'
  | 'InvalidFacet'
  | 'InvalidDatatypeDefinition'
  | 'TopDataPropertyMisuse'
  | 'ExtractionNote'
  | 'InsufficientOperands';

/** One OWL 2 DL profile violation, with the input triples it is about. */
export interface OwlDlViolation {
  kind: OwlDlViolationKind;
  /** A human-readable explanation naming the entities involved. */
  detail: string;
  triples: OwlDlTriple[];
}

/** A global restriction the profile check did not verify (so `profile.dl` is not `true`). */
export interface OwlDlUnchecked {
  rule: string;
  triples: OwlDlTriple[];
}

/** The OWL 2 DL profile check. `dl` is `true` only with no violation and nothing unchecked. */
export interface OwlDlProfile {
  dl: boolean | 'unknown';
  violations: OwlDlViolation[];
  unchecked: OwlDlUnchecked[];
}

/** Why the reasoner gave no verdict on a parsed ontology. */
export type OwlDlUnknownKind =
  | 'OutOfFragment'
  | 'NotInDl'
  | 'ResourceBudget'
  | 'ImportsNotLoaded'
  | 'DatatypeUnknown'
  | 'Unconfirmed';

/** Why the RDF graph could not be read as an OWL 2 ontology (the fail-closed reverse mapping). */
export type OwlDlRefusalKind = 'Unparsed' | 'Malformed' | 'Ambiguous' | 'Unsupported';

/** The reason attached to `consistent: "unknown"`. */
export type OwlDlReason =
  /** A construct outside the decided fragment (e.g. `owl:topObjectProperty`). */
  | { kind: 'OutOfFragment'; construct: string; triples: OwlDlTriple[] }
  /** A blocking OWL 2 DL violation (a non-simple property where a simple one is required, or an irregular property hierarchy). */
  | { kind: 'NotInDl'; violations: OwlDlViolation[] }
  /** The deterministic work budget ran out. */
  | { kind: 'ResourceBudget' }
  /** A datatype question the reasoner does not decide (e.g. an XSD pattern with a block escape). */
  | { kind: 'DatatypeUnknown'; triples: OwlDlTriple[] }
  /**
   * A candidate model the reasoner could not confirm: nominals tie a repeated part of it to
   * an individual whose number restrictions the repetition could break, or a key compares
   * data values not fixed to literals.
   */
  | { kind: 'Unconfirmed' }
  /** Consistent on its own, but `owl:imports` targets were not loaded. */
  | { kind: 'ImportsNotLoaded'; imports: string[] }
  /** The graph is not read as an OWL 2 ontology at all. */
  | { kind: OwlDlRefusalKind; detail: string; triples: OwlDlTriple[] };

/** The result of {@link checkOwlDl}. */
export interface OwlDlResult {
  /** Consistency of TBox plus ABox. */
  consistent: boolean | 'unknown';
  /** `null` unless `consistent` is `"unknown"`. */
  reason: OwlDlReason | null;
  profile: OwlDlProfile;
  /** The unsatisfiable named classes (IRIs), sorted. Empty unless `classification` is `"complete"`. */
  unsatisfiable: string[];
  /** Entailed `[sub, super]` named-class subsumptions that are not asserted, sorted. Empty unless `classification` is `"complete"`. */
  inferredSubClassOf: [string, string][];
  /** `"complete"` when classified; `{ unknown }` when consistent but not classified; `null` when not consistent. */
  classification: 'complete' | { unknown: OwlDlUnknownKind } | null;
}

/** Deterministic work bounds of the reasoner (non-negative integers; omitted = default). */
export interface OwlDlBudget {
  /** Rule applications (facts added, nodes created, branches opened). */
  maxSteps?: number;
  /** Nodes of the completion graph (at most 2^32 - 1). */
  maxNodes?: number;
}

/** Options for {@link checkOwlDl}. */
/** The RDF syntaxes {@link checkOwlDl} reads. */
export type OwlDlFormat = 'turtle' | 'ntriples' | 'nquads' | 'trig';

export interface OwlDlOptions {
  /** The syntax of both documents (default `'turtle'`). Named graphs are folded into the default graph. */
  format?: OwlDlFormat;
  budget?: OwlDlBudget;
}

type DlModule = typeof import('../wasm-dl/sparq_dl_wasm.js');

declare const process: { versions?: { node?: string } } | undefined;

let loading: Promise<DlModule> | undefined;

/** Imports and initialises the DL bundle once (retried if an attempt fails). */
function loadDl(): Promise<DlModule> {
  if (!loading) {
    loading = (async () => {
      const mod = await import('../wasm-dl/sparq_dl_wasm.js');
      if (typeof process !== 'undefined' && process?.versions?.node) {
        const { readFile } = await import('node:fs/promises');
        const bytes = await readFile(new URL('../wasm-dl/sparq_dl_wasm_bg.wasm', import.meta.url));
        await mod.default({ module_or_path: bytes });
      } else {
        await mod.default();
      }
      return mod;
    })();
    loading.catch(() => {
      loading = undefined;
    });
  }
  return loading;
}

function budgetLimit(name: string, value: unknown, max = Number.MAX_SAFE_INTEGER): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0 || value > max) {
    throw new TypeError(`checkOwlDl: options.budget.${name} must be an integer from 0 to ${max}`);
  }
  return value;
}

/** `maxNodes` is a `usize` in the bundle, which is 32-bit on wasm32. */
const MAX_NODES = 2 ** 32 - 1;

const FORMATS: readonly string[] = ['turtle', 'ntriples', 'nquads', 'trig'] satisfies OwlDlFormat[];

/**
 * The own values of `value`, a plain object of this realm (prototype `Object.prototype` or
 * `null`) whose own properties are all enumerable string-keyed data properties named in
 * `allowed`; anything else is a TypeError. The values are taken from the property descriptors,
 * so no getter runs and nothing inherited is consulted.
 */
function plainObject(value: unknown, allowed: readonly string[], what: string): Record<string, unknown> {
  const proto = typeof value === 'object' && value !== null ? Object.getPrototypeOf(value) : undefined;
  if (proto !== Object.prototype && proto !== null) {
    throw new TypeError(`checkOwlDl: ${what} must be a plain object`);
  }
  const values: Record<string, unknown> = Object.create(null);
  for (const key of Reflect.ownKeys(value as object)) {
    const d = Object.getOwnPropertyDescriptor(value, key);
    if (typeof key !== 'string' || !allowed.includes(key)) {
      throw new TypeError(`checkOwlDl: unknown ${what} key ${String(key)} (expected ${allowed.join(', ')})`);
    }
    if (!d || !d.enumerable || !('value' in d)) {
      throw new TypeError(`checkOwlDl: ${what}.${key} must be a plain enumerable value`);
    }
    values[key] = d.value;
  }
  return values;
}

/**
 * Validates `options` strictly (an object with only the `allowed` keys; a known `format`; a
 * `budget` object with only non-negative integer limits) and returns the JSON the bundle takes,
 * so a malformed configuration is a `TypeError`, never a silent fallback to the defaults.
 * @internal
 */
export function normaliseOwlDlOptions(
  options: unknown,
  allowed: readonly string[] = ['format', 'budget'],
): { format: OwlDlFormat; budget: OwlDlBudget | undefined; optionsJson: string | undefined } {
  if (options === undefined) return { format: 'turtle', budget: undefined, optionsJson: undefined };
  const opts = plainObject(options, allowed, 'options');
  const rawFormat = opts.format;
  const format = rawFormat === undefined ? 'turtle' : rawFormat;
  if (typeof format !== 'string' || !FORMATS.includes(format)) {
    throw new TypeError(`checkOwlDl: options.format must be one of ${FORMATS.join(', ')}`);
  }
  const rawBudget = opts.budget;
  if (rawBudget === undefined) return { format: format as OwlDlFormat, budget: undefined, optionsJson: undefined };
  const b = plainObject(rawBudget, ['maxSteps', 'maxNodes'], 'options.budget');
  const budget: OwlDlBudget = {};
  const maxSteps = budgetLimit('maxSteps', b.maxSteps);
  const maxNodes = budgetLimit('maxNodes', b.maxNodes, MAX_NODES);
  if (maxSteps !== undefined) budget.maxSteps = maxSteps;
  if (maxNodes !== undefined) budget.maxNodes = maxNodes;
  return { format: format as OwlDlFormat, budget, optionsJson: JSON.stringify({ budget }) };
}

/**
 * Checks an OWL 2 ontology: parses `tbox` and the optional `abox` (merged into one graph; the
 * ABox is just more assertions, and blank nodes never join across the two documents), reads
 * the graph as OWL 2, runs the OWL 2 DL profile check, consistency, and — for a consistent
 * ontology — classification.
 *
 * Decided fragment: SROIQ(D) without the top and bottom properties — classes and every
 * class constructor including nominals, object properties with inverses, hierarchies,
 * transitivity and regular property chains, the role characteristics, `hasSelf`, qualified
 * cardinality up to a fixed bound, data properties over the OWL 2 datatype map, keys over
 * named individuals, and the ABox. Keys on a non-simple property, the top and bottom
 * properties, datatypes outside the map and object/data property punning answer
 * `consistent: "unknown"` with reason `OutOfFragment` and the triples; an undecided datatype
 * question answers `DatatypeUnknown`, and an unconfirmed model with nominals `Unconfirmed`.
 * Options must be plain objects of this realm.
 *
 * Rejects on a parse error, on malformed options, or if the DL bundle cannot be loaded. A graph
 * the OWL mapping refuses is NOT a rejection: it resolves with `consistent: "unknown"`.
 */
export async function checkOwlDl(tbox: string, abox?: string, options?: OwlDlOptions): Promise<OwlDlResult> {
  if (typeof tbox !== 'string') throw new TypeError('checkOwlDl: tbox must be a string');
  if (abox !== undefined && abox !== null && typeof abox !== 'string') {
    throw new TypeError('checkOwlDl: abox must be a string when given');
  }
  const { format, optionsJson } = normaliseOwlDlOptions(options);
  const mod = await loadDl();
  return JSON.parse(mod.checkOwlDl(tbox, abox ?? undefined, format, optionsJson)) as OwlDlResult;
}

/** What {@link OwlDlSession.check} reused from the session's previous check. */
export type OwlDlReuse = 'none' | 'identical' | 'extension';

/** The result of {@link OwlDlSession.check}: a {@link OwlDlResult} plus what was reused. */
export interface OwlDlSessionResult extends OwlDlResult {
  /**
   * `"identical"`: the same axioms as the last decided check, answered from it without
   * reasoning. `"extension"`: those axioms and more, so the last inconsistency, unsatisfiable
   * classes and subsumptions carry over (OWL 2 is monotone) and only the rest is tested.
   * `"none"`: a full check (the first, an axiom removed or changed, or nothing decided kept).
   */
  reuse: OwlDlReuse;
}

/**
 * Rechecks successive versions of an ontology (a working draft plus small edits), keeping the
 * last decided answer to reuse (a refused or undecided version keeps it). Results are what
 * {@link checkOwlDl} would answer, except that a reused answer may be decided where a fresh
 * check would run out of budget or abstain. The axioms, declarations and imports are
 * compared by the terms they name; an anonymous individual by its blank-node label, so a
 * relabelled blank node counts as a change. Call {@link OwlDlSession.free} when done.
 */
export interface OwlDlSession {
  /** As {@link checkOwlDl}, synchronously (the bundle is already loaded). */
  check(tbox: string, abox?: string, options?: OwlDlOptions): OwlDlSessionResult;
  /** Releases the session's memory in the bundle; the session cannot be used afterwards. */
  free(): void;
}

/**
 * Opens an {@link OwlDlSession}, loading the DL bundle on first use as {@link checkOwlDl} does.
 * Rejects if the bundle cannot be loaded.
 */
export async function openOwlDlSession(): Promise<OwlDlSession> {
  const mod = await loadDl();
  let inner: InstanceType<DlModule['OwlDlSession']> | undefined = new mod.OwlDlSession();
  return {
    check(tbox: string, abox?: string, options?: OwlDlOptions): OwlDlSessionResult {
      if (!inner) throw new TypeError('OwlDlSession: used after free()');
      if (typeof tbox !== 'string') throw new TypeError('OwlDlSession.check: tbox must be a string');
      if (abox !== undefined && abox !== null && typeof abox !== 'string') {
        throw new TypeError('OwlDlSession.check: abox must be a string when given');
      }
      const { format, optionsJson } = normaliseOwlDlOptions(options);
      return JSON.parse(inner.check(tbox, abox ?? undefined, format, optionsJson)) as OwlDlSessionResult;
    },
    free(): void {
      inner?.free();
      inner = undefined;
    },
  };
}
