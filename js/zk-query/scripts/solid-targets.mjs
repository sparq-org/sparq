// Research grade and not externally audited. Synthetic Solid target fixtures only,
// ported from the query crate's native target driver: same facts, issuers, trust
// members, holder binding, status entries, validity and creation instants.
import { testIssuerKeygen, testIssue, testHolderKeygen, statusSnapshot } from '../index.js';
const XSD = 'http://www.w3.org/2001/XMLSchema#';
const CRED = 'https://www.w3.org/2018/credentials#';
const STATUS = 'https://www.w3.org/ns/credentials/status#';
const RECYCLED = ['urn:fictional:recycled-content:tier', 'urn:fictional:recycled-content:tier-2', 'urn:fictional:recycled-content:tier-3'];
export const instant = Date.parse('2026-11-13T00:00:00Z');
const integer = (p, v) => `<urn:subject> <${p}> "${v}"^^<${XSD}integer> .\n`;
const date = (p, v) => `<urn:subject> <${p}> "${v}"^^<${XSD}date> .\n`;
/** [issuer IRI, claim N-Quads] per credential, in slot order. */
export function facts(target, kase = 'honest') {
  if (target === '1a' || target === '1b') {
    const licence = integer('urn:claim:licence.years', kase === 'threshold' ? 4 : 5) + integer('urn:claim:licence.points', kase === 'points' ? 4 : 3);
    let register;
    if (target === '1a') register = integer('urn:claim:claims.five-year-count', kase === 'claim' ? 1 : 0) + `<urn:subject> <urn:claim:claims.five-year-complete> "${kase !== 'incomplete'}"^^<${XSD}boolean> .\n`;
    else {
      register = date('https://w3id.org/jeswr/awp#periodStart', kase === 'start' ? '2021-11-14' : '2021-11-13') + date('https://w3id.org/jeswr/awp#periodEnd', kase === 'period' ? '2026-11-12' : '2026-11-13');
      if (kase === 'claim' || kase === 'old-claim') register += `<urn:subject> <urn:claim:claims.claim> <urn:claim:entry> .\n<urn:claim:entry> <urn:claim:claims.date> "${kase === 'old-claim' ? '2020-01-01' : '2026-11-01'}"^^<${XSD}date> .\n`;
    }
    return [['urn:fictional:insurance-quote:licensing', licence], ['urn:fictional:insurance-quote:claims-register', register]];
  }
  if (target === '2') {
    let r = integer('urn:claim:trial.age', kase === 'threshold' ? 17 : kase === 'upper' ? 66 : 18);
    r += `<urn:subject> <urn:claim:trial.diagnosis> "${kase === 'diagnosis' ? 'condition-b' : 'condition-a'}" .\n`;
    r += date('https://w3id.org/jeswr/awp#periodStart', kase === 'start' ? '2026-11-02' : '2026-11-01');
    r += date('https://w3id.org/jeswr/awp#periodEnd', kase === 'period' ? '2026-11-12' : '2026-11-13');
    for (let i = 0; i < (kase === 'no-medications' ? 0 : 32); i++) r += `<urn:subject> <urn:claim:trial.medications> "${kase === 'excluded' && i === 31 ? 'excluded-x' : `allowed-${i}`}" .\n`;
    return [['urn:fictional:trial-eligibility:practice', r]];
  }
  const fraction = target === '3';
  return RECYCLED.map((issuer, i) => {
    const value = fraction ? (kase === 'threshold' && i === 2 ? '0.29' : ['0.30', '0.40', '0.50'][i]) : (kase === 'threshold' && i === 2 ? '110.0' : ['200.0', '180.0', '150.0'][i]);
    return [issuer, `<urn:fictional:recycled-content:lot> <urn:claim:material.${fraction ? 'recycled-fraction' : 'recycled-mass'}> "${value}"^^<${XSD}decimal> .\n`];
  });
}
const recycled = target => target.startsWith('3');
/**
 * Credentials, policy envelope, holder and status lists for one target case.
 * `revoked` sets the status bit; the others keep honest facts.
 */
export function targetFixture(target, kase = 'honest') {
  const holder = testHolderKeygen('17'.repeat(32));
  const rows = facts(target, kase);
  const keyIndex = (issuer, i) => recycled(target) ? RECYCLED.indexOf(issuer) : i;
  const issuers = new Map(), lists = new Map();
  const credentials = rows.map(([issuer, claims], i) => {
    const k = keyIndex(issuer, i);
    const key = testIssuerKeygen((k + 0x40).toString(16).repeat(32));
    issuers.set(k, { issuer, verificationMethod: `urn:method:${k}`, publicKey: key.publicKey });
    lists.set(k, { list: `urn:list:${k}`, purpose: 'revocation', bits: kase === 'revoked' ? '1' : '0' });
    let graph = `${claims}<urn:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <${CRED}VerifiableCredential> .
<urn:credential> <${CRED}issuer> <${issuer}> .
<urn:credential> <${CRED}credentialSubject> <urn:subject> .
<urn:credential> <${CRED}validFrom> "2020-01-01T00:00:00Z"^^<${XSD}dateTime> .
<urn:credential> <${CRED}validUntil> "2030-01-01T00:00:00Z"^^<${XSD}dateTime> .
<urn:credential> <${CRED}credentialStatus> <urn:entry> .
<urn:entry> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <${STATUS}BitstringStatusListEntry> .
<urn:entry> <${STATUS}statusListCredential> <urn:list:${k}> .
<urn:entry> <${STATUS}statusPurpose> "revocation" .
<urn:entry> <${STATUS}statusListIndex> "0" .
`;
    if (!recycled(target)) graph += `<urn:subject> <${CRED}confidenceMethod> <urn:holder:key> .
<urn:holder:key> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://w3id.org/security#Multikey> .
<urn:holder:key> <https://w3id.org/security#publicKeyMultibase> "${holder.multikey}" .
`;
    const proofConfigNQuads = `<urn:proof> <https://w3id.org/security#cryptosuite> "schnorr-poseidon2-merkle-2026" .
<urn:proof> <https://w3id.org/security#verificationMethod> <urn:method:${k}> .
<urn:proof> <http://purl.org/dc/terms/created> "2026-11-13T00:00:00Z"^^<${XSD}dateTime> .
`;
    return testIssue({ issuer: key, graphNQuads: graph, seedHex: (i + 0x60).toString(16).repeat(32), status: { proofConfigNQuads } });
  });
  // The verifier accepts the honest (unset) snapshot of each issuer's list.
  const members = [...issuers.values()];
  const accepted = [...lists.values()].map(l => statusSnapshot({ ...l, bits: '0' }).digest);
  const policy = {
    trustedIssuers: members.map(m => m.publicKey), trustMembers: members,
    acceptedSnapshots: { digests: accepted }, validityTime: instant, earliestCreated: instant,
    ...(recycled(target) ? {} : { holderBinding: {} }),
  };
  return { credentials, policy, holder: recycled(target) ? undefined : holder, statusLists: [...lists.values()] };
}
/** The catalogue entry whose example is this target's query, by case name. */
export const targetNames = { '1a': 'target-insurance-1a', '1b': 'target-insurance-1b', '2': 'target-trial', '3': 'target-recycled-min', '3-sum': 'target-recycled-sum', '3-having': 'target-recycled-having' };
export function targetEntry(table, target) {
  return table.entries.find(e => e.example.name === targetNames[target]);
}
