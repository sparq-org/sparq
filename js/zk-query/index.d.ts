/** Research grade and not externally audited. */
export interface ProductionIssuer { secretKeyHex: string; publicKey: string }
export interface TestIssuer { seedHex: string; publicKey: string }
export interface ProofConfiguration { proofConfigNQuads: string }
export interface Credential {
  graphNQuads: string; proofConfigNQuads: string; saltHex: string; rootHex: string;
  count: number; issuer: string; signatureHex: string;
}
export interface Bounds { credentials: number; quadsPerCredential: number; rowsPerTable: number }
export interface TrustMember { issuer: string; verificationMethod: string; publicKey: string }
export interface Policy {
  bounds: Bounds; trustedIssuers: string[]; trustMembers: TrustMember[]; audience: string;
  notBefore?: number; notAfter?: number; credentialQuadBounds?: number[];
  inputCommitment?: string; maxPresentationBytes?: number; maxResultSize?: number;
  holderBinding?: unknown; acceptedSnapshots?: unknown; validityTime?: number; earliestCreated?: number;
}
export interface Request {
  query: string; policy: Policy; nonce: string; planHash: string;
  verificationKey: string; digest: string; requestRdf: string;
}
export type QueryResult = { boolean: boolean } | { variables: string[]; rows: (string | null)[][] };
export interface Presentation {
  requestDigest: string; inputCommitment: string; signatureMode: number;
  result: QueryResult; proof: string;
}
export interface VerificationReport {
  rule: { query: string; planHash: string }; issuers: string[]; subject: string[];
  fresh: string; complete: true; issuerRoot: string; trustMembers: TrustMember[];
}
export type Verification =
  | { verified: true; released: QueryResult; report: VerificationReport }
  | { verified: false; released: []; reason: string; report?: never };
export interface ChallengeStore { consume(nonce: string): 'fresh' | 'alreadyConsumed' }
export type Backend = 'native' | 'wasm' | { type: 'native'; nargoPath?: string; bbPath?: string; presentationSaltHex?: string } | { type: 'wasm'; presentationSaltHex?: string };
export function issuerKeygen(): ProductionIssuer;
export function testIssuerKeygen(seedHex: string): TestIssuer;
export function issue(input: { issuer: ProductionIssuer | TestIssuer; graphNQuads: string; status?: ProofConfiguration }): Credential;
export function testIssue(input: { issuer: TestIssuer; graphNQuads: string; status?: ProofConfiguration; seedHex: string }): Credential;
export function request(input: { query: string; policy: Policy; nonce: string }): Request;
export function prove(input: { request: Request; credentials: Credential[]; backend?: Backend }): Promise<{ presentation: Presentation; result: QueryResult }>;
export function verify(input: { request: Request; presentation: Presentation; seenNonces: ChallengeStore | Set<string> }): Promise<Verification>;
