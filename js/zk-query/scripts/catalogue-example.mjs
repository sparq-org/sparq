// Research grade and not externally audited. One policy per catalogue example.
// Envelope values (snapshot digests, instants) are public inputs, not plan shape, so
// placeholders derive the same plan as any verifier's real values.
export const trustMembers = issuer => [{ issuer: 'urn:issuer', verificationMethod: 'urn:method', publicKey: issuer.publicKey }];
export function examplePolicy(example, issuer, audience) {
  const { holder, status, validity, freshness, tableRowBounds, openingProfile } = example.envelope ?? {};
  return {
    bounds: example.bounds,
    ...(example.credentialQuadBounds === undefined ? {} : { credentialQuadBounds: example.credentialQuadBounds }),
    trustedIssuers: [issuer.publicKey], trustMembers: trustMembers(issuer), audience,
    ...(holder ? { holderBinding: {} } : {}),
    ...(status ? { acceptedSnapshots: { digests: ['01'.repeat(32)] } } : {}),
    ...(validity ? { validityTime: 0 } : {}),
    ...(freshness ? { earliestCreated: 0 } : {}),
    ...(tableRowBounds ? { tableRowBounds } : {}),
    ...(openingProfile ? { openingProfile } : {}),
  };
}
/** The fields a catalogue entry records for its example. */
export const exampleRecord = example => ({
  query: example.query, bounds: example.bounds,
  ...(example.credentialQuadBounds === undefined ? {} : { credentialQuadBounds: example.credentialQuadBounds }),
  ...(example.envelope === undefined ? {} : { envelope: example.envelope }),
});
