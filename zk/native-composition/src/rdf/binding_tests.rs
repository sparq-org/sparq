//! [GPT-6] Genuine weaker-statement controls for local synthetic binding fixtures.
//!
//! These helpers intentionally bypass honest support preparation. A successful
//! construction is never acceptance of the requested RDF relation.

use super::*;

/// Construct and verify a proof that omits the requested RDF preimages.
///
/// This test-only path admits one role and one row. It uses the exact required
/// claim context and nonce, but discloses only the signed protocol and status.
/// The returned bytes have passed independent verification of that weaker
/// statement. Callers must submit them to `verify_public_bgp` and require the
/// typed `Verification` failure before recording a negative control.
///
/// # Errors
/// Returns profile/capacity errors for unsupported test requests and
/// `Construction` if real proof generation or weaker verification fails.
pub fn prove_without_query_preimages<R: RngCore + CryptoRng>(
    rng: &mut R,
    request: &Request,
    credential: &Credential,
) -> Result<Presentation> {
    if request.roles.len() != 1 || request.rows.len() != 1 {
        return Err(Error::Capacity);
    }
    let required = required_lines(request)?;
    // Unique bounded slots make the required public statement well-formed even
    // when the requested triples are absent. They are not claimed as actual
    // credential support; only the verifier can authenticate that relationship.
    let support = Support {
        rows: vec![(0..required[0].len())
            .map(|triple| Slot { role: 0, triple })
            .collect()],
    };
    let (mut disclosed, context) = relation(request, &support)?;
    let public = &mut disclosed[0];
    public.retain(|index, _| *index < 2);
    let mut prover = Statements::new();
    prover.add(PoKBBSSignatureG1Prover::new_statement_from_params(
        parameters(),
        public.clone(),
    ));
    let mut verifier = Statements::new();
    verifier.add(PoKBBSSignatureG1Verifier::new_statement_from_params(
        parameters(),
        request.roles[0].issuer.key.clone(),
        public.clone(),
    ));
    let mut witnesses = Witnesses::new();
    witnesses.add(PoKBBSSignatureG1::new_as_witness(
        credential.signature.clone(),
        credential
            .messages
            .iter()
            .enumerate()
            .filter(|(index, _)| !public.contains_key(index))
            .map(|(index, message)| (index, *message))
            .collect(),
    ));
    let prover = ProofSpec::new(prover, MetaStatements::new(), vec![], Some(context.clone()));
    let verifier = ProofSpec::new(verifier, MetaStatements::new(), vec![], Some(context));
    prover.validate().map_err(|_| Error::Construction)?;
    verifier.validate().map_err(|_| Error::Construction)?;
    let (proof, _) = Proof::new::<R, Blake2b512>(
        rng,
        prover,
        witnesses,
        Some(request.nonce.to_vec()),
        Default::default(),
    )
    .map_err(|_| Error::Construction)?;
    proof
        .clone()
        .verify::<R, Blake2b512>(
            rng,
            verifier,
            Some(request.nonce.to_vec()),
            Default::default(),
        )
        .map_err(|_| Error::Construction)?;
    Ok(Presentation {
        support,
        proof: encode(proof)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_std::rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn weaker_control_requires_actual_verification_under_expected_issuer() {
        let mut rng = StdRng::seed_from_u64(482);
        let issuer = Issuer::generate(&mut rng, "urn:fixture:issuer").unwrap();
        let other = Issuer::generate(&mut rng, "urn:fixture:other").unwrap();
        let reference = StatusReference {
            list: "urn:fixture:status".into(),
            epoch: 1,
            index: 0,
        };
        let credential =
            issue_rdf(&mut rng, &issuer, "<urn:a> <urn:p> <urn:b> .", &reference).unwrap();
        let mut request = Request {
            query: "SELECT DISTINCT ?s ?o WHERE { ?s <urn:p> ?o }".into(),
            rows: vec![Mapping::from([
                ("s".into(), "<urn:absent>".into()),
                ("o".into(), "<urn:b>".into()),
            ])],
            nonce: [93; 32],
            roles: vec![RolePolicy {
                issuer: issuer.public(),
                status: AcceptedStatus {
                    reference,
                    bits: vec![0],
                },
            }],
        };
        let weaker = prove_without_query_preimages(&mut rng, &request, &credential).unwrap();
        assert_eq!(
            verify_public_bgp(&mut rng, &request, &weaker, &mut ConsumedNonces::default()),
            Err(Error::Verification)
        );
        request.roles[0].issuer = other.public();
        assert_eq!(
            prove_without_query_preimages(&mut rng, &request, &credential).unwrap_err(),
            Error::Construction
        );
        request.rows.push(request.rows[0].clone());
        assert_eq!(
            prove_without_query_preimages(&mut rng, &request, &credential).unwrap_err(),
            Error::Capacity
        );
    }
}
