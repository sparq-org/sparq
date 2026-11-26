// Research grade and not externally audited. Synthetic coverage authority only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { coverageDocument, request, testIssue, testIssuerKeygen } from '../index.js';
import { canonicalGraph, issueFixture } from '../scripts/test-credentials.mjs';

const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
const fixture = JSON.parse(readFileSync(new URL('./fixtures/authenticated.json', import.meta.url)));
const catalogue = JSON.parse(readFileSync(new URL('../assets/catalogue.json', import.meta.url)));
const entry = catalogue.entries.find(e => e.example.name === 'join-two-credentials');
const query = entry.example.query;
const verificationKey = readFileSync(new URL(`../assets/${entry.vk}`, import.meta.url), 'utf8').trim();
const nonce = fixture.request.nonce;
const sender = testIssuerKeygen('00'.repeat(32));
const signer = testIssuerKeygen('10'.repeat(32));
const other = testIssuerKeygen('11'.repeat(32));
const authority = { issuer: 'urn:registry:authority', verificationMethod: 'urn:registry:key', publicKey: signer.publicKey };
const scope = {
  registry: 'urn:registry', subject: 'urn:a', claimTypes: ['urn:p', 'urn:q'],
  periodStart: 1, periodEnd: 9, method: 'urn:inventory', profile: 'urn:inventory:v1',
};
const policy = { ...fixture.request.policy, bounds: entry.example.bounds, notBefore: 10, notAfter: 20 };
const salt = '03'.repeat(32);
const credentials = [
  issueFixture(sender, canonicalGraph('<urn:a> <urn:p> <urn:b> .\n<urn:a> <urn:q> <urn:c> .\n'), '01'.repeat(32)),
  issueFixture(sender, canonicalGraph('<urn:a> <urn:r> <urn:d> .\n'), '02'.repeat(32)),
];

const call = (operation, input) => JSON.parse(host.hostCall(operation, JSON.stringify(input)));
const store = (policy, clock = 10) => call('request', { query, policy, nonce, verificationKey, clock });
const prepare = (request, credentials, clock = 10) => call('prepareProof', {
  request, credentials, clock, presentationSaltHex: salt,
});
const verify = (request, presentation, proof = () => true, consume = () => 'fresh', clock = 10) =>
  JSON.parse(host.verifyWith(JSON.stringify({ request, presentation }), proof, consume, () => clock));

function issue(claims, issuer = signer, seedHex = '13'.repeat(32)) {
  return testIssue({ issuer, ...coverageDocument(claims), seedHex });
}
function scenario() {
  const open = store(policy);
  const prepared = prepare(open, credentials);
  const inputCommitment = prepared.presentationSkeleton.inputCommitment;
  const { issuer, verificationMethod } = authority;
  const claims = { issuer, verificationMethod, scope, asOf: 10, expiresAt: 20, inputCommitment };
  const anchored = { ...policy, inputCommitment };
  const covered = { ...anchored, coverage: { authority, scope, credential: issue(claims) } };
  return { open, prepared, claims, anchored, covered };
}

test('coverage binds the canonical signed claims to the public request statement', () => {
  const { open, prepared, claims, anchored, covered } = scenario();
  const stored = store(covered);
  const p = prepare(stored, credentials);
  assert.deepEqual(p.result, { variables: ['s'], rows: [['<urn:a>']] });
  assert.deepEqual(p.result, prepared.result);
  assert.equal(p.presentationSkeleton.inputCommitment, claims.inputCommitment);
  assert.equal(stored.planHash, open.planHash);
  assert.equal(stored.verificationKey, open.verificationKey);
  const plain = store(anchored);
  const plainPrepared = prepare(plain, credentials);
  assert.notEqual(stored.digest, plain.digest);
  assert.match(stored.requestRdf, /vcq#coverageDigest>/);
  assert.notDeepEqual(p.publicInputs, plainPrepared.publicInputs);

  const presentation = { ...p.presentationSkeleton, proof: '01' };
  const checked = verify(stored, presentation, text => {
    assert.deepEqual(JSON.parse(text).publicInputs, p.publicInputs);
    return true;
  });
  assert.equal(checked.verified, true, checked.reason);
  assert.equal(checked.report.complete, true);
  assert.equal(checked.report.exactness, 'agreed-dataset');
  const { digest, ...report } = checked.report.coverage;
  const canonicalClaims = covered.coverage.credential.graphNQuads.trimEnd().split('\n').sort().join('\n') + '\n';
  assert.equal(digest, createHash('sha256').update(canonicalClaims).digest('hex'));
  assert.deepEqual(report, claims);
  assert.ok(stored.requestRdf.includes(`"u${Buffer.from(digest, 'hex').toString('base64url')}"`));

  const reissued = issue({ ...claims, scope: { ...scope, claimTypes: [...scope.claimTypes].reverse() } },
    signer, '14'.repeat(32));
  assert.notEqual(reissued.signatureHex, covered.coverage.credential.signatureHex);
  const canonical = store({ ...covered, coverage: { ...covered.coverage, credential: reissued } });
  assert.equal(canonical.digest, stored.digest);
  const invalidSignature = structuredClone(stored);
  invalidSignature.policy.coverage.credential.signatureHex = reissued.signatureHex;
  const invalid = verify(invalidSignature, presentation,
    () => assert.fail('coverage signature must be checked during reconstruction'),
    () => assert.fail('invalid coverage must leave the challenge unused'));
  assert.equal(invalid.verified, false);
  assert.match(invalid.reason, /coverage signature/);
  const extended = store({ ...covered, coverage: { ...covered.coverage, credential: issue({ ...claims, expiresAt: 21 }) } });
  assert.notEqual(extended.digest, stored.digest);

  const stripped = structuredClone(stored);
  delete stripped.policy.coverage;
  const refused = verify(stripped, presentation, () => assert.fail('coverage is absent'));
  assert.equal(refused.verified, false);
  assert.match(refused.reason, /stored request reconstruction differs/);
  assert.equal(plain.coverageCheckedAt, undefined);

  const plainChecked = verify(plain, { ...plainPrepared.presentationSkeleton, proof: '01' });
  assert.equal(plainChecked.report.exactness, 'agreed-dataset');
  assert.equal(plainChecked.report.coverage, undefined);
  const openChecked = verify(open, { ...prepared.presentationSkeleton, proof: '01' });
  assert.equal(openChecked.report.exactness, 'presented-dataset');
  assert.equal(openChecked.report.complete, true);
  assert.equal(openChecked.report.coverage, undefined);
});

test('request refuses independently changed coverage signer, scope, period, dataset and signature', () => {
  const { claims, covered } = scenario();
  const withCredential = credential => ({ ...covered, coverage: { ...covered.coverage, credential } });
  const rejects = (change, reason, issuer) => assert.throws(
    () => store(withCredential(issue({ ...claims, ...change }, issuer))), reason,
  );
  rejects({}, /coverage signer key/, other);
  rejects({ issuer: 'urn:other' }, /coverage signer/);
  rejects({ verificationMethod: 'urn:other' }, /coverage signer/);
  rejects({ scope: { ...scope, registry: 'urn:other' } }, /coverage scope/);
  rejects({ scope: { ...scope, subject: 'urn:other' } }, /coverage scope/);
  rejects({ scope: { ...scope, claimTypes: ['urn:p'] } }, /coverage scope/);
  rejects({ scope: { ...scope, periodStart: 2 } }, /coverage period/);
  rejects({ scope: { ...scope, periodEnd: 8 } }, /coverage period/);
  rejects({ scope: { ...scope, method: 'urn:other' } }, /coverage method/);
  rejects({ scope: { ...scope, profile: 'urn:other' } }, /coverage profile/);
  rejects({ inputCommitment: '55'.repeat(32) }, /coverage dataset commitment/);
  assert.throws(() => store({ ...covered, inputCommitment: undefined }), /coverage dataset commitment required/);
  const signedByOther = issue(claims, other);
  assert.throws(() => store(withCredential({ ...signedByOther, issuer: signer.publicKey })), /coverage signature/);
  const fields = coverageDocument(claims);
  const wrongMethodConfig = { proofConfigNQuads: fields.status.proofConfigNQuads.replace(authority.verificationMethod, 'urn:other') };
  const wrongMethod = testIssue({ issuer: signer, ...fields, status: wrongMethodConfig, seedHex: '15'.repeat(32) });
  assert.throws(() => store(withCredential(wrongMethod)), /coverage signer verification method/);
  const extra = testIssue({ issuer: signer, ...fields, seedHex: '16'.repeat(32),
    graphNQuads: fields.graphNQuads + '_:coverage <urn:extra> "unexpected" .\n' });
  assert.throws(() => store(withCredential(extra)), /coverage document/);
});

test('request creation validates coverage instants and caps the default request end', () => {
  const { claims, anchored, covered } = scenario();
  const defaults = { ...covered, notBefore: undefined, notAfter: undefined };
  const stored = store(defaults);
  assert.equal(stored.coverageCheckedAt, 10);
  assert.equal(stored.policy.notBefore, claims.asOf);
  assert.equal(stored.policy.notAfter, claims.expiresAt);
  assert.throws(() => store({ ...covered, notBefore: 9 }), /coverage validity window/);
  assert.throws(() => store({ ...covered, notAfter: 21 }), /coverage validity window/);
  assert.throws(() => store(defaults, 9), /coverage validity at request creation/);
  assert.throws(() => store(defaults, 20), /coverage validity at request creation/);

  const future = { ...covered, notBefore: 12, notAfter: 18 };
  assert.throws(() => store(future, 9), /coverage validity at request creation/);
  const scheduled = store(future, 10);
  assert.equal(scheduled.policy.notBefore, 12);
  const presentation = { ...prepare(scheduled, credentials, 12).presentationSkeleton, proof: '01' };
  const seen = () => assert.fail('coverage must be revalidated before proof or nonce handling');
  const { coverageCheckedAt, ...withoutCheckedAt } = scheduled;
  assert.equal(coverageCheckedAt, 10);
  assert.match(verify(withoutCheckedAt, presentation, seen, seen, 12).reason,
    /stored request reconstruction differs/);
  for (const invalid of [9, 20]) {
    assert.match(verify({ ...scheduled, coverageCheckedAt: invalid }, presentation, seen, seen, 12).reason,
      /coverage validity at request creation/);
  }

  const now = Math.floor(Date.now() / 1000);
  const live = issue({ ...claims, expiresAt: now + 300 });
  const js = request({ query, nonce, policy: {
    ...anchored, notBefore: undefined, notAfter: undefined, coverage: { authority, scope, credential: live },
  } });
  assert.equal(js.policy.notAfter, now + 300);
  const p = prepare(stored, credentials);
  const refused = verify(stored, { ...p.presentationSkeleton, proof: '01' },
    () => assert.fail('expired'), () => assert.fail('expired'), 20);
  assert.equal(refused.verified, false);
  assert.match(refused.reason, /request validity window/);
});

test('coverage issuance checks historical intervals and claim bounds', () => {
  const { claims, covered } = scenario();
  assert.throws(() => coverageDocument({ ...claims, scope: { ...scope, periodEnd: 11 } }), /coverage period/);
  assert.throws(() => coverageDocument({ ...claims, expiresAt: claims.asOf }), /coverage validity/);
  assert.throws(() => coverageDocument({ ...claims, scope: { ...scope, claimTypes: [] } }), /coverage scope claim types/);
  assert.throws(() => coverageDocument({ ...claims, scope: {
    ...scope, claimTypes: Array.from({ length: 65 }, (_, i) => `urn:claim:${i}`),
  } }), /coverage scope claim types/);
  const large = structuredClone(covered);
  large.coverage.credential.graphNQuads += ' '.repeat(65_536);
  assert.throws(() => store(large), /coverage document size/);
});

test('request admission checks the signed historical interval', () => {
  const { claims, covered } = scenario();
  const fields = coverageDocument(claims);
  const graphNQuads = fields.graphNQuads.replace(/(coveragePeriodEnd> )"9"/, '$1"11"');
  assert.notEqual(graphNQuads, fields.graphNQuads);
  const credential = testIssue({ issuer: signer, ...fields, graphNQuads, seedHex: '17'.repeat(32) });
  assert.throws(() => store({ ...covered, coverage: {
    authority, credential, scope: { ...scope, periodEnd: 11 },
  } }), /coverage period/);
});

test('omitting a dataset credential fails agreement even when the complete answer is equal', () => {
  const { open, prepared, covered } = scenario();
  const stored = store(covered);
  const omitted = prepare(open, credentials.slice(0, 1));
  assert.deepEqual(omitted.result, prepared.result);
  assert.notEqual(omitted.presentationSkeleton.inputCommitment, covered.inputCommitment);
  assert.throws(() => prepare(stored, credentials.slice(0, 1)), /dataset commitment agreement/);
  const refused = verify(stored, {
    ...omitted.presentationSkeleton, requestDigest: stored.digest, proof: '01',
  }, () => assert.fail('wrong dataset'), () => assert.fail('wrong dataset'));
  assert.equal(refused.verified, false);
  assert.match(refused.reason, /dataset commitment agreement/);
});
