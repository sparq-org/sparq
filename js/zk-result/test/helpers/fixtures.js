import { randomBytes } from 'node:crypto';
import { testIssue, testIssuerKeygen } from '../../index.js';

export const query = `PREFIX ex: <https://example.org/>
SELECT DISTINCT ?p ?age WHERE {
  ?p ex:age ?age .
  ?p ex:degree ex:BSc .
  ?p ex:identity.verified true .
  FILTER (?age >= 21)
}`;
export function fixture(two = false) {
  const first = testIssuerKeygen('a1');
  const second = testIssuerKeygen('b2');
  const status = { status_list: 'https://example.org/status', version: 1, index: 0 };
  const age = '<https://example.org/alice> <https://example.org/age> "30"^^<http://www.w3.org/2001/XMLSchema#integer> .\n';
  const degree = '<https://example.org/alice> <https://example.org/degree> <https://example.org/BSc> .\n';
  const identity = '<https://example.org/alice> <https://example.org/identity.verified> "true"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n';
  const issue = (issuer, graphNQuads, status) => testIssue({ issuer, graphNQuads, status, seedHex: 'c3' });
  const credentials = two
    ? [issue(first, age, status), issue(second, degree + identity, { ...status, index: 1 })]
    : [issue(first, age + degree + identity, status)];
  return {
    query, credentials,
    policy: { trustedIssuers: [first.publicKey, second.publicKey], snapshots: [{ status_list: status.status_list, version: 1, bits: [0] }], minVersion: 1, maxVersion: 1 },
    nonce: '0x' + randomBytes(31).toString('hex'),
  };
}
