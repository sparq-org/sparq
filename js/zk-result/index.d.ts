export type ReleasedTerm = { kind: 'iri'; value: string } | {
  kind: 'literal'; value: string; datatype?: string; language?: string;
};
export type ReleasedRow = Record<string, ReleasedTerm>;
/** Research-grade key. The caller must protect and store the secret; the package never persists it. */
export interface Issuer { secretKeyHex: string; publicKey: string; seedHex?: never }
export interface TestIssuer { seedHex: string; publicKey: string; secretKeyHex?: never }
export interface StatusRef { status_list: string; version: number; index: number }
export interface Credential {
  graphNQuads: string; salt: string; issuer: string; signature: string; statusRef: StatusRef;
}
export interface TrustMember { issuer: string; verificationMethod: string; publicKey: string }
export interface Policy {
  /** Trusted keys; when trustMembers is also supplied their key sets must be equal. */
  trustedIssuers?: string[];
  /** Trusted issuer IRI to compressed public key; overlaps with trustMembers must agree. */
  issuers?: Record<string, string>;
  /** Absolute issuer/method IRIs and canonical keys. A scoped issuer IRI must map to one distinct key. */
  trustMembers?: TrustMember[];
  snapshots: { status_list: string; version: number; bits: number[] }[];
  minVersion: number; maxVersion: number;
}
export interface Presentation {
  version: number; query: string; rows: ReleasedRow[]; challenge: string;
  issuer_slots: string[]; integer_capacity?: 'two_digits' | 'full_u64'; proof: number[];
}
/** Synchronous shared store. The caller owns persistence and request binding. */
export interface SeenNonces { has(nonce: string): boolean; add(nonce: string): unknown }
/** Deterministic hexadecimal u64 seed; test issuance only. */
export function testIssuerKeygen(seedHex: string): TestIssuer;
/** Platform CSPRNG issuer key; research grade, not externally audited. Never persisted by the package. */
export function issuerKeygen(): Issuer;
/** Fresh platform CSPRNG salt and method-bound issuer attestation. Rejects mixed key formats. */
export function issue(input: { issuer: Issuer | TestIssuer; graphNQuads: string; status: StatusRef }): Credential;
/** Accepted older positional form; also uses a fresh platform CSPRNG salt. */
export function issue(issuer: Issuer | TestIssuer, graphNQuads: string, statusRef: StatusRef): Credential;
/** Test-only deterministic salt from a hexadecimal u64 seed, issuer public key and exact graph UTF-8 bytes.
 * Refuses production keys: a predictable salt lets anyone who can guess the graph test a guess against the commitment.
 */
export function testIssue(input: { issuer: TestIssuer; graphNQuads: string; status: StatusRef; seedHex: string }): Credential;
export interface VerificationReport {
  rule: { query: string; member: string };
  /** Scoped: the single requested policy IRI; every authenticated slot has its key.
   * Unscoped: each slot's policy IRIs (sorted), or key hex if unnamed; repeats remain.
   * A slot need not contribute a fact; there is no per-fact attribution across issuers.
   */
  issuers: string[];
  /** Canonical sorted, deduplicated policy roster, present only when supplied. */
  trustMembers?: TrustMember[];
  /** This statement has no holder binding. */
  subject: [];
  /** Canonical nonce, returned only after successful store consumption. */
  fresh: string;
  /** Selected solution support does not establish that none is missing. */
  complete: false;
}
export type VerificationResult = { verified: true; released: ReleasedRow[]; report: VerificationReport; reason?: never } |
  { verified: false; released: ReleasedRow[]; reason: string; report?: never };
export function prove(input: { query: string; credentials: Credential[]; policy: Policy; nonce: string }):
  Promise<{ presentation: Presentation; result: ReleasedRow[] }>;
export function verify(input: {
  expectedQuery: string; presentation: Presentation; policy: Policy; nonce: string; seenNonces: SeenNonces;
}): Promise<VerificationResult>;
