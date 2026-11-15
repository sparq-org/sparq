import { randomBytes } from 'node:crypto';
import { issue, testIssuerKeygen } from '../../index.js';

export function filterFixture(type, two = false, depth = 10) {
  const first = testIssuerKeygen('a1');
  const second = testIssuerKeygen('b2');
  const status = { status_list: 'https://example.org/status', version: 1, index: 0 };
  const date = type === 'date';
  const predicate = date ? 'birthDate' : 'balance';
  const value = date ? '2000-02-29' : '18446744073709551615';
  const datatype = `http://www.w3.org/2001/XMLSchema#${date ? 'date' : 'integer'}`;
  const bound = date ? '"2008-11-13"^^xsd:date' : '18446744073709551614';
  const operand = `<https://example.org/alice> <https://example.org/${predicate}> "${value}"^^<${datatype}> .\n`;
  const degree = '<https://example.org/alice> <https://example.org/degree> <https://example.org/BSc> .\n';
  // A second candidate must be excluded by the hidden comparison.
  const excluded = `<https://example.org/bob> <https://example.org/${predicate}> "${date ? '2009-01-01' : '20'}"^^<${datatype}> .\n` +
    '<https://example.org/bob> <https://example.org/degree> <https://example.org/BSc> .\n';
  const credentials = two
    ? [issue(first, operand + excluded, status), issue(second, degree, { ...status, index: 1 })]
    : [issue(first, operand + degree + excluded, status)];
  const snapshotBytes = depth === 10 ? 1 : depth === 17 ? 129 : 16385;
  return {
    query: `PREFIX ex: <https://example.org/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
SELECT DISTINCT ?p WHERE {
  ?p ex:${predicate} ?value . ?p ex:degree ex:BSc .
  FILTER (?value ${date ? '<=' : '>'} ${bound})
}`,
    credentials,
    policy: { trustedIssuers: [first.publicKey, second.publicKey], snapshots: [{ status_list: status.status_list, version: 1, bits: new Array(snapshotBytes).fill(0) }], minVersion: 1, maxVersion: 1 },
    nonce: '0x' + randomBytes(31).toString('hex'),
  };
}
