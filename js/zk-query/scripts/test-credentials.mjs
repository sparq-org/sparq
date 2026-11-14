// Research grade and not externally audited. Synthetic credential fixtures only.
import { testIssue } from '../index.js';
export const trustMembers = issuer => [{ issuer: 'urn:issuer', verificationMethod: 'urn:method', publicKey: issuer.publicKey }];
export const canonicalGraph = graph => '<urn:vc> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n<urn:vc> <https://www.w3.org/2018/credentials#issuer> <urn:issuer> .\n' + graph;
export const issueFixture = (issuer, graphNQuads, seedHex) => testIssue({ issuer, graphNQuads, seedHex, status: { proofConfigNQuads: '_:proof <https://w3id.org/security#cryptosuite> "schnorr-poseidon2-merkle-2026" .\n_:proof <https://w3id.org/security#verificationMethod> <urn:method> .\n' } });
