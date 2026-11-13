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
export interface Policy {
  trustedIssuers: string[];
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
export function issue(issuer: Issuer | TestIssuer, graphNQuads: string, statusRef: StatusRef): Credential;
export function prove(input: { query: string; credentials: Credential[]; policy: Policy; nonce: string }):
  Promise<{ presentation: Presentation; result: ReleasedRow[] }>;
export function verify(input: {
  expectedQuery: string; presentation: Presentation; policy: Policy; nonce: string; seenNonces: SeenNonces;
}): Promise<{ verified: boolean; released: ReleasedRow[]; reason?: string }>;
