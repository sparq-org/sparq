/* tslint:disable */
/* eslint-disable */

/**
 * SHA-256 of the canonical member key embedded at build time from the shared registry.
 */
export function canonicalVkHash(member: string): string;

/**
 * Issues a string-canonical credential with a fresh platform-CSPRNG salt and method-bound attestation.
 */
export function issue(issuer_json: string, graph_n_quads: string, status_ref_json: string): string;

/**
 * Prepares private ABI inputs and the public skeleton using the shared result relation.
 */
export function prepare(query_text: string, credentials_json: string[], policy_json: string, nonce_hex: string, options_json: string): string;

/**
 * Independently reconstructs the expected statement; it performs no proof check.
 */
export function reconstructPublicInputs(expected_query_text: string, presentation_json: string, policy_json: string, nonce_hex: string): string;

/**
 * Deterministic issuer convenience for tests only; never production key generation.
 */
export function testIssuerKeygen(seed_hex: string): string;
