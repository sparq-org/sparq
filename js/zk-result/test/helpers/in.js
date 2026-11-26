import { testIssuerKeygen, testIssue } from '../../index.js';

export function inFixture(two = false, value = 'BSc') {
  const issuer = testIssuerKeygen('a1');
  const iri = 'urn:fictional:degree-recognition:university';
  const status = { status_list: 'urn:list', version: 1, index: 0 };
  const degree = `<urn:subject> <urn:claim:degree.level> "${value}" .\n`;
  const flag = '<urn:subject> <urn:claim:degree.approved> "true"^^<http://www.w3.org/2001/XMLSchema#boolean> .\n';
  const issue = (graphNQuads, index) => testIssue({ issuer, graphNQuads, status: { ...status, index }, seedHex: 'c3' });
  return {
    query: `PREFIX vcq: <https://w3id.org/sparq/vcq#>
SELECT DISTINCT ?subject WHERE {
  ?g vcq:verifiedIssuer <${iri}> .
  ${two ? `?h vcq:verifiedIssuer <${iri}> .` : ''}
  GRAPH ?g { ?subject <urn:claim:degree.level> ?value .
    FILTER(?value IN ("BSc", "MSc")) }
  ${two ? 'GRAPH ?h { ?subject <urn:claim:degree.approved> true . }' : ''}
}`,
    credentials: two ? [issue(degree, 0), issue(flag, 1)] : [issue(degree, 0)],
    policy: { issuers: { [iri]: issuer.publicKey }, snapshots: [{ status_list: 'urn:list', version: 1, bits: [0] }], minVersion: 1, maxVersion: 1 },
    nonce: '0x789a',
  };
}
