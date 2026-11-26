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
export interface CoverageScope {
  registry: string; subject: string; claimTypes: string[];
  /** Half-open historical interval in Unix seconds. */
  periodStart: number; periodEnd: number;
  method: string; profile: string;
}
export interface CoverageClaims {
  issuer: string; verificationMethod: string; scope: CoverageScope; inputCommitment: string;
  /** Unix seconds. The credential is valid from asOf until expiresAt, exclusive. */
  asOf: number; expiresAt: number;
}
export interface CoveragePolicy {
  /** Authority independently authorized by the verifier for this scope. */
  authority: TrustMember;
  scope: CoverageScope;
  credential: Credential;
}
export interface Policy {
  bounds: Bounds; trustedIssuers: string[]; trustMembers: TrustMember[]; audience: string;
  notBefore?: number; notAfter?: number; credentialQuadBounds?: number[];
  inputCommitment?: string; maxPresentationBytes?: number; maxResultSize?: number;
  /** Detached coverage evidence for the explicit inputCommitment. */
  coverage?: CoveragePolicy;
  /** Holder binding: omit `issuers` to bind every credential; a present list is nonempty. */
  holderBinding?: { issuers?: string[] };
  /** Snapshot digests from `statusSnapshot`; every credential status entry must open in one. */
  acceptedSnapshots?: { digests: string[]; allowMissing?: boolean };
  /** Unix epoch milliseconds: credentials must be valid at this instant. */
  validityTime?: number;
  /** Unix epoch milliseconds: credential proofs must be created at or after this instant. */
  earliestCreated?: number;
  /** Row limits, one per plan node in plan order, committed in the plan hash. */
  tableRowBounds?: number[];
  /** Lexical opening limits, default `solid`; holder keys need `long-objects`. */
  openingProfile?: 'solid' | 'long-objects' | 'extended';
}
export interface HolderKey { secretKeyHex: string; publicKey: string; multikey: string }
/** One published status list: one character per entry, "0" valid and "1" set. */
export interface StatusList { list: string; purpose: string; bits: string }
export interface Request {
  query: string; policy: Policy; nonce: string; planHash: string;
  verificationKey: string; digest: string; requestRdf: string;
  /** Verifier-recorded request creation time in Unix seconds, present with coverage. */
  coverageCheckedAt?: number;
}
export type QueryResult = { boolean: boolean } | { variables: string[]; rows: (string | null)[][] };
export interface Presentation {
  requestDigest: string; inputCommitment: string; signatureMode: number;
  result: QueryResult; proof: string;
}
export interface VerificationReport {
  rule: { query: string; planHash: string };
  /** The whole trusted public-key set relied on for membership authorization, not the
   *  keys that signed the presented credentials (unlike `@sparq-org/zk-result`, whose
   *  `issuers` lists the slots the proof used). */
  issuers: string[];
  /** Issuers whose credentials had to be bound to the presenting holder. */
  subject: string[];
  fresh: string;
  /** Complete relative to the presented signed roots only (closed world). */
  complete: true;
  exactness: 'presented-dataset' | 'agreed-dataset';
  coverage?: CoverageClaims & { digest: string };
  issuerRoot: string; trustMembers: TrustMember[];
  /** Present only when enforced. */
  status?: { acceptedSnapshots: string[]; allowMissing: boolean };
  validityTime?: number; earliestCreated?: number; tableRowBounds?: number[];
}
export type Verification =
  | { verified: true; released: QueryResult; report: VerificationReport }
  | { verified: false; released: []; reason: string; report?: never };
export interface ChallengeStore { consume(nonce: string): 'fresh' | 'alreadyConsumed' }
export type Backend = 'native' | 'wasm' | { type: 'native'; nargoPath?: string; bbPath?: string; presentationSaltHex?: string } | { type: 'wasm'; presentationSaltHex?: string };
export function issuerKeygen(): ProductionIssuer;
export function testIssuerKeygen(seedHex: string): TestIssuer;
export function holderKeygen(): HolderKey;
export function testHolderKeygen(seedHex: string): HolderKey;
export function statusSnapshot(list: StatusList): { digest: string; root: string; depth: number; bitLength: number };
export function issue(input: { issuer: ProductionIssuer | TestIssuer; graphNQuads: string; status?: ProofConfiguration }): Credential;
export function testIssue(input: { issuer: TestIssuer; graphNQuads: string; status?: ProofConfiguration; seedHex: string }): Credential;
export function coverageDocument(claims: CoverageClaims): { graphNQuads: string; status: ProofConfiguration };
export function request(input: { query: string; policy: Policy; nonce: string }): Request;
export function prove(input: { request: Request; credentials: Credential[]; holder?: { secretKeyHex: string }; statusLists?: StatusList[]; backend?: Backend }): Promise<{ presentation: Presentation; result: QueryResult }>;
export function verify(input: { request: Request; presentation: Presentation; seenNonces: ChallengeStore | Set<string> }): Promise<Verification>;
