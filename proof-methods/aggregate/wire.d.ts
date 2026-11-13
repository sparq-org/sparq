/** Unpadded base64url bytes; points are compressed Ristretto and scalars are canonical. */
export type Bytes = string;
export interface Party {
  party_iri: string;
  ed25519_public_key: Bytes;
  x25519_public_key: Bytes;
  computeOnly?: boolean;
}
export interface Threshold {
  statistic: 'count' | 'sum';
  op: '>=' | '<';
  /** Canonical unsigned decimal text. */
  value: string;
}
export interface SessionParams {
  session_id: Bytes;
  statistic: 'count-and-sum' | 'count-only';
  min_group: number;
  min_contributors?: number;
  max_value_bits?: number;
  threshold?: Threshold;
  roster: Party[];
}
export interface Session { parameters: SessionParams; rule_digest: Bytes }
export interface SecretKeys { signing_secret: Bytes; encryption_secret: Bytes }
export interface KeyPair {
  secret: SecretKeys;
  ed25519_public_key: Bytes;
  x25519_public_key: Bytes;
}
export interface Input { count: number; sum?: string | null }
export interface Commitments { count: Bytes; sum?: Bytes }
export interface Ciphertext { ephemeral_key: Bytes; nonce: Bytes; ciphertext: Bytes }
export interface Contribution {
  session_id: Bytes;
  rule_digest: Bytes;
  index: number;
  commitment: Commitments;
  range_proof: Bytes;
  share_commitments: Commitments[];
  ciphertexts: (Ciphertext | null)[];
  signature: Bytes;
}
/** Contributor-private openings: never include in recipient messages or logs. */
export interface Shares { count: Bytes; rho: Bytes; sum?: Bytes; sigma?: Bytes }
export interface LocalShare { session_id: Bytes; rule_digest: Bytes; index: number; shares: Shares }
export interface ContributionState { contribution: Contribution; local: LocalShare; audit_opening: Shares }
/** Contributor-private signed plaintext. */
export interface Partial extends LocalShare { contributions_digest: Bytes; signature: Bytes }
export interface PartialMessage {
  session_id: Bytes;
  rule_digest: Bytes;
  contributions_digest: Bytes;
  index: number;
  recipient: number;
  packet: Ciphertext;
}
export interface PartialDispatch { messages: PartialMessage[]; local: Partial }
export interface Release { kind: 'release'; count: number; rho: Bytes; sum?: string; sigma?: Bytes }
export interface ThresholdVerdict { kind: 'threshold-verdict'; verdict: boolean; limb_commitments: Bytes[]; range_proof: Bytes }
export interface BelowMinimumGroup { kind: 'below-minimum-group'; range_proof: Bytes }
export interface TooFewContributors { kind: 'too-few-contributors' }
export type Outcome = Release | ThresholdVerdict | BelowMinimumGroup | TooFewContributors;
export interface SignedOutcome {
  session_id: Bytes;
  rule_digest: Bytes;
  contributions_digest: Bytes;
  finisher: number;
  outcome: Outcome;
  signature: Bytes;
}
export interface VerifyResult { verified: true }
export interface ProtocolError { code: string; index: number | null; check: string }
export interface PartyInput { keys: SecretKeys; input: Input }
/** Simulation only: includes private partials from every party. */
export interface Simulation { session: Session; contributions: Contribution[]; partials: PartialDispatch[]; outcome: SignedOutcome }
