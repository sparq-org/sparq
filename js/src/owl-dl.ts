/**
 * OWL 2 DL reasoning: consistency, the OWL 2 DL profile check, unsatisfiable classes and the
 * inferred class hierarchy, from the `sparq-reason-dl` hypertableau reasoner compiled to its
 * OWN wasm bundle (`wasm-dl/`, built from `crates/sparq-dl-wasm`).
 *
 * The bundle is loaded LAZILY: nothing here imports it statically, so a page that imports
 * `@sparq-org/sparq` but never calls {@link checkOwlDl} never fetches it. The first call
 * imports `../wasm-dl/sparq_dl_wasm.js` dynamically and initialises it (in Node from the bytes
 * on disk, elsewhere by `fetch` relative to the module); the load is memoised and retried
 * after a failure.
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
export type OwlDlUnknownKind = 'OutOfFragment' | 'NotInDl' | 'ResourceBudget' | 'ImportsNotLoaded';

/** Why the RDF graph could not be read as an OWL 2 ontology (the fail-closed reverse mapping). */
export type OwlDlRefusalKind = 'Unparsed' | 'Malformed' | 'Ambiguous' | 'Unsupported';

/** The reason attached to `consistent: "unknown"`. */
export type OwlDlReason =
  /** A construct outside the decided fragment (e.g. a data property or a nominal). */
  | { kind: 'OutOfFragment'; construct: string; triples: OwlDlTriple[] }
  /** A blocking OWL 2 DL violation (a non-simple property where a simple one is required, or an irregular property hierarchy). */
  | { kind: 'NotInDl'; violations: OwlDlViolation[] }
  /** The deterministic work budget ran out. */
  | { kind: 'ResourceBudget' }
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
  /** Nodes of the completion graph. */
  maxNodes?: number;
}

/** Options for {@link checkOwlDl}. */
export interface OwlDlOptions {
  /** The syntax of both documents (default `'turtle'`). Named graphs are folded into the default graph. */
  format?: 'turtle' | 'ntriples' | 'nquads' | 'trig';
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

function budgetLimit(name: string, value: unknown): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new TypeError(`checkOwlDl: options.budget.${name} must be a non-negative integer`);
  }
  return value;
}

/**
 * Checks an OWL 2 ontology: parses `tbox` and the optional `abox` (merged into one graph; the
 * ABox is just more assertions, and blank nodes never join across the two documents), reads
 * the graph as OWL 2, runs the OWL 2 DL profile check, consistency, and — for a consistent
 * ontology — classification.
 *
 * Decided fragment: classes and class constructors except nominals, object properties with
 * inverses, hierarchies, transitivity, `R∘R ⊑ R` chains, the role characteristics, `hasSelf`,
 * qualified cardinality, and the ABox. Data properties, datatypes, nominals, other property
 * chains and keys answer `consistent: "unknown"` with reason `OutOfFragment` and the triples.
 *
 * Rejects on a parse error, on malformed options, or if the DL bundle cannot be loaded. A graph
 * the OWL mapping refuses is NOT a rejection: it resolves with `consistent: "unknown"`.
 */
export async function checkOwlDl(tbox: string, abox?: string, options: OwlDlOptions = {}): Promise<OwlDlResult> {
  if (typeof tbox !== 'string') throw new TypeError('checkOwlDl: tbox must be a string');
  if (abox !== undefined && abox !== null && typeof abox !== 'string') {
    throw new TypeError('checkOwlDl: abox must be a string when given');
  }
  const format = options.format ?? 'turtle';
  const budget = options.budget
    ? { maxSteps: budgetLimit('maxSteps', options.budget.maxSteps), maxNodes: budgetLimit('maxNodes', options.budget.maxNodes) }
    : undefined;
  const optionsJson = budget ? JSON.stringify({ budget }) : undefined;
  const mod = await loadDl();
  return JSON.parse(mod.checkOwlDl(tbox, abox ?? undefined, format, optionsJson)) as OwlDlResult;
}
