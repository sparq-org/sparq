use super::*;
use std::sync::OnceLock;

#[derive(Clone)]
struct Fixture {
    session: Session,
    keys: Vec<KeyPair>,
    states: Vec<ContributionState>,
    contributions: Vec<Contribution>,
    partials: Vec<PartialDispatch>,
    output: SignedOutcome,
}
impl Fixture {
    fn new(n: usize, sums: bool, minimum: u32, bits: u8) -> Self {
        let keys: Vec<_> = (0..n)
            .map(|i| generate_keys(Some([i as u8 + 1; 32])))
            .collect();
        let session = Session::new(Parameters {
            session_id: Bytes(vec![minimum as u8 + bits; 32]),
            statistic: if sums {
                Statistic::CountAndSum
            } else {
                Statistic::CountOnly
            },
            min_group: minimum,
            min_contributors: 3,
            max_value_bits: bits,
            threshold: None,
            roster: keys
                .iter()
                .enumerate()
                .map(|(i, k)| Party {
                    party_iri: format!("urn:party:{i}"),
                    compute_only: false,
                    ed25519_public_key: k.ed25519_public_key.clone(),
                    x25519_public_key: k.x25519_public_key.clone(),
                })
                .collect(),
        })
        .unwrap();
        let states: Vec<_> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                contribute(
                    &session,
                    i as u32,
                    &k.secret,
                    &Input {
                        count: 2,
                        sum: sums.then(|| (10 + i).to_string()),
                    },
                )
                .unwrap()
            })
            .collect();
        let contributions = states
            .iter()
            .map(|s| s.contribution.clone())
            .collect::<Vec<_>>();
        let partials = if n >= 3 {
            keys.iter()
                .enumerate()
                .map(|(i, k)| {
                    receive_and_partial(
                        &session,
                        i as u32,
                        &k.secret,
                        &contributions,
                        &states[i].local,
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };
        let messages = partials
            .iter()
            .flat_map(|p| p.messages.iter().filter(|m| m.recipient == 0).cloned())
            .collect::<Vec<_>>();
        let output = finish(
            &session,
            0,
            &keys[0].secret,
            &contributions,
            partials.first().map(|p| &p.local),
            &messages,
        )
        .unwrap();
        Self {
            session,
            keys,
            states,
            contributions,
            partials,
            output,
        }
    }
    fn receive(&self, cs: &[Contribution]) -> Result<PartialDispatch> {
        receive_and_partial(
            &self.session,
            0,
            &self.keys[0].secret,
            cs,
            &self.states[0].local,
        )
    }
    fn messages(&self, recipient: u32) -> Vec<PartialMessage> {
        self.partials
            .iter()
            .flat_map(|p| {
                p.messages
                    .iter()
                    .filter(|m| m.recipient == recipient)
                    .cloned()
            })
            .collect()
    }
    fn finish(
        &self,
        local: Option<&Partial>,
        messages: &[PartialMessage],
    ) -> Result<SignedOutcome> {
        finish(
            &self.session,
            0,
            &self.keys[0].secret,
            &self.contributions,
            local,
            messages,
        )
    }
    fn resign_contribution(&self, c: &mut Contribution) {
        c.signature = sign(&self.keys[c.index as usize].secret, &contribution_bytes(c)).unwrap();
    }
    fn resign_output(&self, o: &mut SignedOutcome) {
        o.signature = sign(&self.keys[o.finisher as usize].secret, &outcome_bytes(o)).unwrap();
    }
    fn verify(&self, o: &SignedOutcome) -> Result<()> {
        verify_outcome(&self.session, &self.contributions, o)
    }
}
fn fixture() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| Fixture::new(3, true, 6, 48))
}
fn refusal() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| Fixture::new(3, true, 7, 48))
}
fn count_only() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| Fixture::new(3, false, 0, 48))
}
fn too_few() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| Fixture::new(2, true, 0, 48))
}
fn reject<T>(result: Result<T>, index: Option<u32>, check: &str) {
    match result {
        Ok(_) => panic!("accepted invalid {check}"),
        Err(e) => {
            assert_eq!(e.index, index, "{e}");
            assert_eq!(e.check, check, "{e}");
        }
    }
}

#[test]
fn full_sessions_and_boundary() {
    for f in [fixture(), count_only(), refusal(), too_few()] {
        f.verify(&f.output).unwrap();
    }
    let six = Fixture::new(6, true, 12, 48);
    six.verify(&six.output).unwrap();
    for f in [fixture(), &six, count_only()] {
        match &f.output.outcome {
            Outcome::Release { count, sum, .. } => {
                assert_eq!(*count, f.keys.len() as u64 * 2);
                assert_eq!(
                    sum,
                    &if f.session.has_sum() {
                        Some(
                            (f.keys.len() * 10 + f.keys.len() * (f.keys.len() - 1) / 2).to_string(),
                        )
                    } else {
                        None
                    }
                );
            }
            _ => panic!("expected release"),
        }
        for i in 0..f.keys.len() {
            let messages = f.messages(i as u32);
            let out = finish(
                &f.session,
                i as u32,
                &f.keys[i].secret,
                &f.contributions,
                Some(&f.partials[i].local),
                &messages,
            )
            .unwrap();
            f.verify(&out).unwrap();
        }
        let json = serde_json::to_string(&f.output).unwrap();
        let roundtrip: SignedOutcome = serde_json::from_str(&json).unwrap();
        f.verify(&roundtrip).unwrap();
    }
    assert!(matches!(
        too_few().output.outcome,
        Outcome::TooFewContributors {}
    ));
    assert!(matches!(
        refusal().output.outcome,
        Outcome::BelowMinimumGroup { .. }
    ));
    let out = serde_json::to_value(&refusal().output).unwrap();
    assert!(out["outcome"].get("count").is_none());
    assert!(out["outcome"].get("rho").is_none());
    assert!(out["outcome"].get("sum").is_none());
    assert!(out["outcome"].get("sigma").is_none());
}

#[test]
fn too_few_refusal_rejects_unsigned_json_fields() {
    let f = too_few();
    f.verify(&f.output).unwrap();
    for (field, value) in [
        ("count", serde_json::json!(0)),
        ("rho", serde_json::json!("")),
        ("sum", serde_json::json!("0")),
        ("sigma", serde_json::json!("")),
        ("range_proof", serde_json::json!("")),
        ("verdict", serde_json::json!(true)),
        ("limb_commitments", serde_json::json!([])),
    ] {
        let mut wire = serde_json::to_value(&f.output).unwrap();
        wire["outcome"][field] = value;
        assert!(
            serde_json::from_value::<SignedOutcome>(wire).is_err(),
            "accepted unsigned refusal field {field}"
        );
    }
}

#[test]
fn contribution_attestation_and_shape_guards() {
    let f = fixture();
    let mut cs = f.contributions.clone();
    cs[1].signature.0[0] ^= 1;
    let e = f.receive(&cs).err().unwrap();
    assert_eq!(e.code, "input-not-attested");
    assert_eq!(e.index, Some(1));
    assert_eq!(e.check, "signature");
    reject(
        verify_outcome(&f.session, &cs, &f.output),
        Some(1),
        "signature",
    );
    let mut cs = f.contributions.clone();
    cs[1].signature = sign(
        &generate_keys(Some([99; 32])).secret,
        &contribution_bytes(&cs[1]),
    )
    .unwrap();
    reject(f.receive(&cs), Some(1), "signature");
    let mut cs = f.contributions.clone();
    cs[1].index = 7;
    reject(f.receive(&cs), Some(7), "roster-index");
    let mut cs = f.contributions.clone();
    cs.push(cs[1].clone());
    reject(f.receive(&cs), Some(1), "duplicate-contributor");
    let mut cs = f.contributions.clone();
    cs.remove(1);
    reject(f.receive(&cs), Some(1), "missing-contributor");
    for check in [
        "session-id",
        "rule-digest",
        "share-count",
        "ciphertext-layout",
        "statistic-shape",
        "point-encoding",
        "count-share-sum",
        "sum-share-sum",
        "range-proof",
    ] {
        let mut cs = f.contributions.clone();
        let c = &mut cs[1];
        match check {
            "session-id" => c.session_id.0[0] ^= 1,
            "rule-digest" => c.rule_digest.0[0] ^= 1,
            "share-count" => {
                c.share_commitments.pop();
            }
            "ciphertext-layout" => c.ciphertexts[0] = None,
            "statistic-shape" => c.commitment.sum = None,
            "point-encoding" => c.commitment.count = Bytes(vec![255; 32]),
            "count-share-sum" => {
                c.share_commitments[0].count =
                    pb(point(&c.share_commitments[0].count, 1).unwrap() + pedersen().B)
            }
            "sum-share-sum" => {
                c.share_commitments[0].sum =
                    Some(pb(point(c.share_commitments[0].sum.as_ref().unwrap(), 1)
                        .unwrap()
                        + pedersen().B))
            }
            "range-proof" => c.range_proof = refusal().contributions[1].range_proof.clone(),
            _ => unreachable!(),
        }
        f.resign_contribution(c);
        reject(f.receive(&cs), Some(1), check);
        reject(verify_outcome(&f.session, &cs, &f.output), Some(1), check);
    }
    let mut reordered = f.contributions.clone();
    reordered.reverse();
    verify_outcome(&f.session, &reordered, &f.output).unwrap();
}

#[test]
fn encrypted_share_guards() {
    let f = fixture();
    for check in [
        "aead",
        "share-commitment",
        "scalar-encoding",
        "statistic-shape",
        "share-encoding",
        "non-contributory-key",
        "nonce",
        "ephemeral-key",
    ] {
        let mut cs = f.contributions.clone();
        let c = &mut cs[1];
        let packet = c.ciphertexts[0].as_mut().unwrap();
        if check == "aead" {
            packet.ciphertext.0[0] ^= 1;
        } else if check == "non-contributory-key" {
            packet.ephemeral_key = Bytes(vec![0; 32]);
        } else if check == "nonce" {
            packet.nonce.0.pop();
        } else if check == "ephemeral-key" {
            packet.ephemeral_key.0.pop();
        } else {
            let bytes = open(
                &f.session,
                b"share",
                1,
                0,
                &Bytes::default(),
                packet,
                &f.keys[0].secret,
            )
            .unwrap();
            let mut shares: Shares = serde_json::from_slice(&bytes).unwrap();
            match check {
                "share-commitment" => {
                    shares.count = sb(scalar(&shares.count, 1).unwrap() + Scalar::ONE)
                }
                "scalar-encoding" => shares.rho = Bytes(vec![255; 32]),
                "statistic-shape" => shares.sigma = None,
                _ => (),
            }
            let bytes = if check == "share-encoding" {
                b"invalid".to_vec()
            } else {
                serde_json::to_vec(&shares).unwrap()
            };
            *packet = seal(
                &f.session,
                b"share",
                1,
                0,
                &Bytes::default(),
                &bytes,
                &mut OsRng,
            )
            .unwrap();
        }
        f.resign_contribution(c);
        reject(f.receive(&cs), Some(1), check);
    }
    let mut local = f.states[0].local.clone();
    local.index = 1;
    reject(
        receive_and_partial(&f.session, 0, &f.keys[0].secret, &f.contributions, &local),
        Some(0),
        "local-index",
    );
    let mut local = f.states[0].local.clone();
    local.shares.count = sb(Scalar::ZERO);
    reject(
        receive_and_partial(&f.session, 0, &f.keys[0].secret, &f.contributions, &local),
        Some(0),
        "share-commitment",
    );
    let f = too_few();
    reject(f.receive(&f.contributions), Some(0), "too-few-contributors");
}

#[test]
fn partial_guards() {
    let f = fixture();
    for check in [
        "partial-commitment",
        "signature",
        "contributions-digest",
        "partial-index",
        "session-id",
        "rule-digest",
        "scalar-encoding",
        "statistic-shape",
    ] {
        let mut messages = f.messages(0);
        let mut p = f.partials[1].local.clone();
        match check {
            "partial-commitment" => {
                p.shares.count = sb(scalar(&p.shares.count, 1).unwrap() + Scalar::ONE)
            }
            "contributions-digest" => p.contributions_digest.0[0] ^= 1,
            "partial-index" => p.index = 2,
            "session-id" => p.session_id.0[0] ^= 1,
            "rule-digest" => p.rule_digest.0[0] ^= 1,
            "scalar-encoding" => p.shares.count = Bytes(vec![255; 32]),
            "statistic-shape" => p.shares.sum = None,
            _ => (),
        }
        // Encrypt under the unchanged envelope context, even when the inner context is wrong.
        p.signature = sign(&f.keys[1].secret, &partial_bytes(&p)).unwrap();
        if check == "signature" {
            p.signature.0[0] ^= 1;
        }
        messages[0].packet = seal(
            &f.session,
            b"partial",
            1,
            0,
            &f.output.contributions_digest,
            &serde_json::to_vec(&p).unwrap(),
            &mut OsRng,
        )
        .unwrap();
        reject(
            f.finish(Some(&f.partials[0].local), &messages),
            Some(1),
            check,
        );
    }
    for check in [
        "duplicate-partial",
        "partial-recipient",
        "contributions-digest",
        "session-id",
        "rule-digest",
        "aead",
        "partial-encoding",
        "roster-index",
        "missing-partial",
    ] {
        let mut messages = f.messages(0);
        match check {
            "duplicate-partial" => messages.push(messages[0].clone()),
            "partial-recipient" => messages[0].recipient = 2,
            "contributions-digest" => messages[0].contributions_digest.0[0] ^= 1,
            "session-id" => messages[0].session_id.0[0] ^= 1,
            "rule-digest" => messages[0].rule_digest.0[0] ^= 1,
            "aead" => messages[0].packet.ciphertext.0[0] ^= 1,
            "partial-encoding" => {
                messages[0].packet = seal(
                    &f.session,
                    b"partial",
                    1,
                    0,
                    &f.output.contributions_digest,
                    b"invalid",
                    &mut OsRng,
                )
                .unwrap()
            }
            "roster-index" => messages[0].index = 8,
            "missing-partial" => {
                messages.remove(0);
            }
            _ => unreachable!(),
        }
        reject(
            f.finish(Some(&f.partials[0].local), &messages),
            Some(if check == "roster-index" { 8 } else { 1 }),
            check,
        );
    }
    reject(f.finish(None, &f.messages(0)), Some(0), "missing-partial");
    reject(
        f.finish(Some(&f.partials[1].local), &f.messages(0)),
        Some(0),
        "local-index",
    );
    let mut local = f.partials[0].local.clone();
    local.shares.rho = sb(Scalar::ZERO);
    local.signature = sign(&f.keys[0].secret, &partial_bytes(&local)).unwrap();
    reject(
        f.finish(Some(&local), &f.messages(0)),
        Some(0),
        "partial-commitment",
    );
}

#[test]
fn output_verifier_guards() {
    let f = fixture();
    for check in [
        "session-id",
        "rule-digest",
        "contributions-digest",
        "signature",
        "roster-index",
        "count-opening",
        "sum-opening",
        "statistic-shape",
        "count-range",
        "sum-range",
        "scalar-encoding",
        "integer-encoding",
    ] {
        let mut out = f.output.clone();
        match check {
            "session-id" => out.session_id.0[0] ^= 1,
            "rule-digest" => out.rule_digest.0[0] ^= 1,
            "contributions-digest" => out.contributions_digest.0[0] ^= 1,
            "roster-index" => out.finisher = 8,
            _ => {
                if let Outcome::Release {
                    count,
                    rho,
                    sum,
                    sigma,
                } = &mut out.outcome
                {
                    match check {
                        "count-opening" => *count += 1,
                        "sum-opening" => *sum = Some("34".into()),
                        "statistic-shape" => *sigma = None,
                        "count-range" => *count = u64::MAX,
                        "sum-range" => *sum = Some(u128::MAX.to_string()),
                        "scalar-encoding" => *rho = Bytes(vec![255; 32]),
                        "integer-encoding" => *sum = Some("033".into()),
                        _ => (),
                    }
                }
            }
        }
        if check != "roster-index" {
            f.resign_output(&mut out);
        }
        if check == "signature" {
            out.signature.0[0] ^= 1;
        }
        reject(
            f.verify(&out),
            Some(if check == "roster-index" { 8 } else { 0 }),
            check,
        );
    }
    let mut out = f.output.clone();
    if let Outcome::Release { rho, sigma, .. } = &mut out.outcome {
        std::mem::swap(rho, sigma.as_mut().unwrap());
    }
    f.resign_output(&mut out);
    reject(f.verify(&out), Some(0), "count-opening");
    let mut out = f.output.clone();
    out.outcome = Outcome::TooFewContributors {};
    f.resign_output(&mut out);
    reject(f.verify(&out), Some(0), "enough-contributors");
    let tf = too_few();
    let mut out = tf.output.clone();
    out.outcome = f.output.outcome.clone();
    tf.resign_output(&mut out);
    reject(tf.verify(&out), Some(0), "too-few-contributors");
    let r = refusal();
    let mut out = r.output.clone();
    out.outcome = f.output.outcome.clone();
    r.resign_output(&mut out);
    reject(r.verify(&out), Some(0), "minimum-group");
    let mut out = f.output.clone();
    out.outcome = r.output.outcome.clone();
    f.resign_output(&mut out);
    reject(f.verify(&out), Some(0), "range-proof");
    let co = count_only();
    let mut out = co.output.clone();
    out.outcome = r.output.outcome.clone();
    co.resign_output(&mut out);
    reject(co.verify(&out), Some(0), "minimum-disabled");
    let mut out = r.output.clone();
    if let Outcome::BelowMinimumGroup { range_proof } = &mut out.outcome {
        range_proof.0[0] ^= 1;
    }
    r.resign_output(&mut out);
    reject(r.verify(&out), Some(0), "range-proof");
}

#[test]
fn session_and_input_guards() {
    let f = fixture();
    for check in [
        "session-id",
        "roster-size",
        "minimum-contributors",
        "value-bits",
        "duplicate-identity",
        "duplicate-signing-key",
        "invalid-encryption-key",
        "signing-key",
        "encryption-key",
    ] {
        let mut p = f.session.parameters.clone();
        match check {
            "session-id" => {
                p.session_id.0.pop();
            }
            "roster-size" => p.roster.clear(),
            "minimum-contributors" => p.min_contributors = 0,
            "value-bits" => p.max_value_bits = 65,
            "duplicate-identity" => p.roster[1].party_iri = p.roster[0].party_iri.clone(),
            "duplicate-signing-key" => {
                p.roster[1].ed25519_public_key = p.roster[0].ed25519_public_key.clone()
            }
            "invalid-encryption-key" => p.roster[1].x25519_public_key = Bytes(vec![0; 32]),
            "signing-key" => {
                p.roster[1].ed25519_public_key.0.pop();
            }
            "encryption-key" => {
                p.roster[1].x25519_public_key.0.pop();
            }
            _ => unreachable!(),
        }
        reject(
            Session::new(p),
            if matches!(
                check,
                "session-id" | "roster-size" | "minimum-contributors" | "value-bits"
            ) {
                None
            } else {
                Some(1)
            },
            check,
        );
    }
    let mut s = f.session.clone();
    s.rule_digest.0[0] ^= 1;
    reject(
        verify_outcome(&s, &f.contributions, &f.output),
        None,
        "rule-digest",
    );
    reject(
        contribute(
            &s,
            0,
            &f.keys[0].secret,
            &Input {
                count: 1,
                sum: Some("1".into()),
            },
        ),
        None,
        "rule-digest",
    );
    for (input, check) in [
        (
            Input {
                count: u32::MAX as u64 + 1,
                sum: Some("0".into()),
            },
            "input-range",
        ),
        (
            Input {
                count: 0,
                sum: Some((1u64 << 48).to_string()),
            },
            "input-range",
        ),
        (
            Input {
                count: 0,
                sum: Some("-1".into()),
            },
            "integer-encoding",
        ),
        (
            Input {
                count: 0,
                sum: None,
            },
            "statistic-shape",
        ),
    ] {
        reject(
            contribute(&f.session, 0, &f.keys[0].secret, &input),
            Some(0),
            check,
        );
    }
    let mut keys = f.keys[0].secret.clone();
    keys.signing_secret = f.keys[1].secret.signing_secret.clone();
    reject(
        contribute(
            &f.session,
            0,
            &keys,
            &Input {
                count: 1,
                sum: Some("1".into()),
            },
        ),
        Some(0),
        "signing-key-mismatch",
    );
    let mut keys = f.keys[0].secret.clone();
    keys.encryption_secret = f.keys[1].secret.encryption_secret.clone();
    reject(
        contribute(
            &f.session,
            0,
            &keys,
            &Input {
                count: 1,
                sum: Some("1".into()),
            },
        ),
        Some(0),
        "encryption-key-mismatch",
    );
}

#[test]
fn malicious_out_of_range_proofs() {
    let f = fixture();
    for bad in [Scalar::from(u32::MAX as u64 + 1), -Scalar::ONE] {
        let mut cs = f.contributions.clone();
        let c = &mut cs[1];
        let r = Scalar::random(&mut OsRng);
        let t = Scalar::random(&mut OsRng);
        c.commitment.count = pb(commit(bad, r));
        c.commitment.sum = Some(pb(commit(Scalar::from(11u64), t)));
        let v = u64::from_le_bytes(bad.to_bytes()[..8].try_into().unwrap());
        // Valid proof bytes for unsigned witnesses cannot open the requested bounded commitments.
        c.range_proof = prove(
            &f.session,
            b"input-range",
            1,
            &[v, 0, 11, max_value(48) - 11],
            &[r, -r, t, -t],
            64,
            &mut OsRng,
        )
        .unwrap();
        f.resign_contribution(c);
        reject(f.receive(&cs), Some(1), "range-proof");
    }
    for bad in [Scalar::from(1u64 << 48), -Scalar::ONE] {
        let mut cs = f.contributions.clone();
        let c = &mut cs[1];
        let (r, t) = (Scalar::random(&mut OsRng), Scalar::random(&mut OsRng));
        c.commitment = Opening {
            c: Scalar::from(2u64),
            r,
            s: bad,
            t,
        }
        .commitments(true);
        let v = u64::from_le_bytes(bad.to_bytes()[..8].try_into().unwrap());
        c.range_proof = prove(
            &f.session,
            b"input-range",
            1,
            &[2, u32::MAX as u64 - 2, v, 0],
            &[r, -r, t, -t],
            64,
            &mut OsRng,
        )
        .unwrap();
        f.resign_contribution(c);
        reject(f.receive(&cs), Some(1), "range-proof");
    }
    let mut out = fixture().output.clone();
    // A finisher can manufacture proof bytes for an unrelated value but cannot prove a false refusal.
    out.outcome = Outcome::BelowMinimumGroup {
        range_proof: prove(
            &f.session,
            b"below-minimum",
            0,
            &[0],
            &[Scalar::ONE],
            32,
            &mut OsRng,
        )
        .unwrap(),
    };
    f.resign_output(&mut out);
    reject(f.verify(&out), Some(0), "range-proof");
}

#[test]
fn share_splitting_properties_through_messages() {
    for sums in [false, true] {
        for n in [1, 2, 3, 6, 9] {
            for _ in 0..8 {
                let value = Opening::random(sums, &mut OsRng);
                let parts = split(&value, n, sums, &mut OsRng);
                let mut combined = Opening::zero();
                let (mut c, mut d) = (RistrettoPoint::identity(), RistrettoPoint::identity());
                for part in parts {
                    let wire = part.wire(sums);
                    let part = Opening::read(&wire, sums, 0).unwrap();
                    combined.add(&part);
                    let points = commitment_points(&part.commitments(sums), sums, 0).unwrap();
                    c += points.0;
                    d += points.1;
                }
                assert_eq!(combined.c, value.c);
                assert_eq!(combined.r, value.r);
                assert_eq!(combined.s, value.s);
                assert_eq!(combined.t, value.t);
                check_opening(&value, &(c, d), sums, 0, "property").unwrap();
            }
        }
    }
    let f = fixture();
    for j in 0..3 {
        let p = receive_and_partial(
            &f.session,
            j,
            &f.keys[j as usize].secret,
            &f.contributions,
            &f.states[j as usize].local,
        )
        .unwrap();
        check_partial(
            &f.session,
            &f.contributions.iter().collect::<Vec<_>>(),
            &f.output.contributions_digest,
            &p.local,
        )
        .unwrap();
    }
}

#[test]
fn unsigned_widths_and_json_precision() {
    for bits in [1, 16, 32, 48, 64] {
        let mut p = fixture().session.parameters.clone();
        p.max_value_bits = bits;
        let s = Session::new(p).unwrap();
        let states: Vec<_> = fixture()
            .keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                contribute(
                    &s,
                    i as u32,
                    &k.secret,
                    &Input {
                        count: u32::MAX as u64,
                        sum: Some(max_value(bits).to_string()),
                    },
                )
                .unwrap()
            })
            .collect();
        let cs: Vec<_> = states.iter().map(|s| s.contribution.clone()).collect();
        let ps: Vec<_> = fixture()
            .keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                receive_and_partial(&s, i as u32, &k.secret, &cs, &states[i].local).unwrap()
            })
            .collect();
        let messages = ps
            .iter()
            .flat_map(|p| p.messages.iter().filter(|m| m.recipient == 0).cloned())
            .collect::<Vec<_>>();
        let out = finish(
            &s,
            0,
            &fixture().keys[0].secret,
            &cs,
            Some(&ps[0].local),
            &messages,
        )
        .unwrap();
        verify_outcome(&s, &cs, &out).unwrap();
        match out.outcome {
            Outcome::Release { count, sum, .. } => {
                assert_eq!(count, 3 * u32::MAX as u64);
                assert_eq!(sum.unwrap(), (3 * max_value(bits) as u128).to_string());
            }
            _ => panic!("expected release"),
        }
    }
    assert!(serde_json::from_str::<Bytes>("\"AA==\"").is_err());
    assert!(serde_json::from_str::<Parameters>("{}").is_err());
    let mut p = serde_json::to_value(&fixture().session.parameters).unwrap();
    p.as_object_mut().unwrap().remove("min_contributors");
    p.as_object_mut().unwrap().remove("max_value_bits");
    let s = Session::new(serde_json::from_value(p).unwrap()).unwrap();
    assert_eq!(s.parameters.min_contributors, 3);
    assert_eq!(s.parameters.max_value_bits, 48);
}

#[test]
fn every_rule_parameter_is_bound_before_verification() {
    let f = fixture();
    for field in [
        "session",
        "statistic",
        "group",
        "contributors",
        "bits",
        "iri",
        "signing",
        "encryption",
        "order",
    ] {
        let mut p = f.session.parameters.clone();
        match field {
            "session" => p.session_id.0[0] ^= 1,
            "statistic" => p.statistic = Statistic::CountOnly,
            "group" => p.min_group = 5,
            "contributors" => p.min_contributors = 2,
            "bits" => p.max_value_bits = 47,
            "iri" => p.roster[1].party_iri.push_str(":changed"),
            "signing" => {
                p.roster[1].ed25519_public_key = generate_keys(Some([77; 32])).ed25519_public_key
            }
            "encryption" => {
                p.roster[1].x25519_public_key = generate_keys(Some([77; 32])).x25519_public_key
            }
            "order" => p.roster.swap(1, 2),
            _ => unreachable!(),
        }
        let session = Session::new(p).unwrap();
        assert_ne!(
            session.rule_digest, f.session.rule_digest,
            "unbound {field}"
        );
        reject(
            verify_outcome(&session, &f.contributions, &f.output),
            Some(0),
            if field == "session" {
                "session-id"
            } else {
                "rule-digest"
            },
        );
    }
}

#[test]
fn signatures_bind_each_wire_component() {
    let f = fixture();
    for field in [
        "commitment",
        "range",
        "share",
        "ciphertext",
        "ephemeral",
        "nonce",
    ] {
        let mut cs = f.contributions.clone();
        let c = &mut cs[1];
        match field {
            "commitment" => c.commitment.count.0[0] ^= 1,
            "range" => c.range_proof.0[0] ^= 1,
            "share" => c.share_commitments[0].count.0[0] ^= 1,
            "ciphertext" => c.ciphertexts[0].as_mut().unwrap().ciphertext.0[0] ^= 1,
            "ephemeral" => c.ciphertexts[0].as_mut().unwrap().ephemeral_key.0[0] ^= 1,
            "nonce" => c.ciphertexts[0].as_mut().unwrap().nonce.0[0] ^= 1,
            _ => unreachable!(),
        }
        reject(f.receive(&cs), Some(1), "signature");
    }
    for field in ["count", "rho", "sum", "sigma"] {
        let mut p = f.partials[1].local.clone();
        match field {
            "count" => p.shares.count = sb(scalar(&p.shares.count, 1).unwrap() + Scalar::ONE),
            "rho" => p.shares.rho = sb(scalar(&p.shares.rho, 1).unwrap() + Scalar::ONE),
            "sum" => {
                p.shares.sum = Some(sb(
                    scalar(p.shares.sum.as_ref().unwrap(), 1).unwrap() + Scalar::ONE
                ))
            }
            "sigma" => {
                p.shares.sigma = Some(sb(
                    scalar(p.shares.sigma.as_ref().unwrap(), 1).unwrap() + Scalar::ONE
                ))
            }
            _ => unreachable!(),
        }
        let mut messages = f.messages(0);
        messages[0].packet = seal(
            &f.session,
            b"partial",
            1,
            0,
            &p.contributions_digest,
            &serde_json::to_vec(&p).unwrap(),
            &mut OsRng,
        )
        .unwrap();
        reject(
            f.finish(Some(&f.partials[0].local), &messages),
            Some(1),
            "signature",
        );
        let mut out = f.output.clone();
        if let Outcome::Release {
            count,
            rho,
            sum,
            sigma,
        } = &mut out.outcome
        {
            match field {
                "count" => *count += 1,
                "rho" => *rho = sb(Scalar::ONE),
                "sum" => *sum = Some("34".into()),
                "sigma" => *sigma = Some(sb(Scalar::ONE)),
                _ => unreachable!(),
            }
        }
        reject(f.verify(&out), Some(0), "signature");
    }
}

#[test]
fn input_proof_transcript_prevents_rebinding_the_same_commitments() {
    let f = fixture();
    for field in ["session", "rules"] {
        let mut p = f.session.parameters.clone();
        if field == "session" {
            p.session_id.0[0] ^= 1;
        } else {
            p.min_group = 5;
        }
        let s = Session::new(p).unwrap();
        let mut cs = f.contributions.clone();
        for c in &mut cs {
            c.session_id = s.parameters.session_id.clone();
            c.rule_digest = s.rule_digest.clone();
            f.resign_contribution(c);
        }
        reject(verify_outcome(&s, &cs, &f.output), Some(0), "range-proof");
    }
    let c = &f.contributions[1];
    let mut total = Opening::zero();
    for j in 0..3 {
        let shares = if j == 1 {
            f.states[1].local.shares.clone()
        } else {
            serde_json::from_slice(
                &open(
                    &f.session,
                    b"share",
                    1,
                    j,
                    &Bytes::default(),
                    c.ciphertexts[j as usize].as_ref().unwrap(),
                    &f.keys[j as usize].secret,
                )
                .unwrap(),
            )
            .unwrap()
        };
        total.add(&Opening::read(&shares, true, 1).unwrap());
    }
    for (purpose, index) in [
        (b"input-range".as_slice(), 2),
        (b"below-minimum".as_slice(), 1),
    ] {
        let mut cs = f.contributions.clone();
        cs[1].range_proof = prove(
            &f.session,
            purpose,
            index,
            &[2, u32::MAX as u64 - 2, 11, max_value(48) - 11],
            &[total.r, -total.r, total.t, -total.t],
            64,
            &mut OsRng,
        )
        .unwrap();
        f.resign_contribution(&mut cs[1]);
        reject(f.receive(&cs), Some(1), "range-proof");
    }
}

#[test]
fn defensive_scalar_conversion_bounds() {
    // These defensive bounds are unreachable from verified range-bounded inputs.
    // Exercise conversion directly as well as the full protocol tests above.
    let mut high = [0; 32];
    high[16] = 1;
    high[0] = 42;
    reject(
        scalar_integer(Scalar::from_bytes_mod_order(high), 100, 4),
        Some(4),
        "aggregate-range",
    );
    reject(
        scalar_integer(-Scalar::ONE, 100, 4),
        Some(4),
        "aggregate-range",
    );
    reject(
        scalar_integer(Scalar::from(101u64), 100, 4),
        Some(4),
        "aggregate-range",
    );
    assert_eq!(scalar_integer(Scalar::from(100u64), 100, 4).unwrap(), 100);
}

#[test]
fn sum_share_and_partial_openings_are_checked() {
    let f = fixture();
    let mut cs = f.contributions.clone();
    let c = &mut cs[1];
    let packet = c.ciphertexts[0].as_mut().unwrap();
    let mut shares: Shares = serde_json::from_slice(
        &open(
            &f.session,
            b"share",
            1,
            0,
            &Bytes::default(),
            packet,
            &f.keys[0].secret,
        )
        .unwrap(),
    )
    .unwrap();
    shares.sum = Some(sb(
        scalar(shares.sum.as_ref().unwrap(), 1).unwrap() + Scalar::ONE
    ));
    *packet = seal(
        &f.session,
        b"share",
        1,
        0,
        &Bytes::default(),
        &serde_json::to_vec(&shares).unwrap(),
        &mut OsRng,
    )
    .unwrap();
    f.resign_contribution(c);
    reject(f.receive(&cs), Some(1), "share-commitment");
    let mut p = f.partials[1].local.clone();
    p.shares.sum = Some(sb(
        scalar(p.shares.sum.as_ref().unwrap(), 1).unwrap() + Scalar::ONE
    ));
    p.signature = sign(&f.keys[1].secret, &partial_bytes(&p)).unwrap();
    let mut messages = f.messages(0);
    messages[0].packet = seal(
        &f.session,
        b"partial",
        1,
        0,
        &p.contributions_digest,
        &serde_json::to_vec(&p).unwrap(),
        &mut OsRng,
    )
    .unwrap();
    reject(
        f.finish(Some(&f.partials[0].local), &messages),
        Some(1),
        "partial-commitment",
    );
    let mut cs = f.contributions.clone();
    let c = &mut cs[1];
    let r = Scalar::random(&mut OsRng);
    let t = Scalar::random(&mut OsRng);
    c.commitment.count = pb(commit(Scalar::from(2u64), r));
    c.commitment.sum = Some(pb(commit(Scalar::from(1u64 << 48), t)));
    c.range_proof = prove(
        &f.session,
        b"input-range",
        1,
        &[2, u32::MAX as u64 - 2, 1u64 << 48, 0],
        &[r, -r, t, -t],
        64,
        &mut OsRng,
    )
    .unwrap();
    f.resign_contribution(c);
    reject(f.receive(&cs), Some(1), "range-proof");
}

#[test]
fn roster_and_count_only_subguards() {
    let f = fixture();
    let mut p = f.session.parameters.clone();
    p.roster[1].party_iri.clear();
    reject(Session::new(p), Some(1), "duplicate-identity");
    let mut p = f.session.parameters.clone();
    p.roster[1].ed25519_public_key = Bytes(vec![0; 32]);
    reject(Session::new(p), Some(1), "duplicate-signing-key");
    let mut p = f.session.parameters.clone();
    p.roster[1].x25519_public_key = p.roster[0].x25519_public_key.clone();
    reject(Session::new(p), Some(1), "invalid-encryption-key");
    let mut p = f.session.parameters.clone();
    p.max_value_bits = 0;
    reject(Session::new(p), None, "value-bits");
    let mut p = f.session.parameters.clone();
    p.roster = vec![p.roster[0].clone(); 1025];
    reject(Session::new(p), None, "roster-size");
    let co = count_only();
    let mut out = co.output.clone();
    if let Outcome::Release { sum, sigma, .. } = &mut out.outcome {
        *sum = Some("0".into());
        *sigma = Some(sb(Scalar::ZERO));
    }
    co.resign_output(&mut out);
    reject(co.verify(&out), Some(0), "statistic-shape");
    let mut local = co.states[0].local.clone();
    local.shares.sum = Some(sb(Scalar::ZERO));
    local.shares.sigma = Some(sb(Scalar::ZERO));
    reject(
        receive_and_partial(
            &co.session,
            0,
            &co.keys[0].secret,
            &co.contributions,
            &local,
        ),
        Some(0),
        "statistic-shape",
    );
    let mut cs = f.contributions.clone();
    cs[1].signature.0.pop();
    reject(f.receive(&cs), Some(1), "signature");
}

#[test]
fn transcript_fields_are_independently_bound() {
    fn challenge(mut t: merlin::Transcript) -> [u8; 32] {
        let mut bytes = [0; 32];
        t.challenge_bytes(b"binding-check", &mut bytes);
        bytes
    }
    let s = &fixture().session;
    let base = challenge(transcript(s, b"input-range", 1));
    let mut changed = s.clone();
    changed.parameters.session_id.0[0] ^= 1;
    assert_ne!(base, challenge(transcript(&changed, b"input-range", 1)));
    let mut changed = s.clone();
    changed.rule_digest.0[0] ^= 1;
    assert_ne!(base, challenge(transcript(&changed, b"input-range", 1)));
    assert_ne!(base, challenge(transcript(s, b"below-minimum", 1)));
    assert_ne!(base, challenge(transcript(s, b"input-range", 2)));
    let uniform: [u8; 64] =
        <sha2::Sha512 as sha2::Digest>::digest(b"sparq-verifiable-aggregate/v1/H").into();
    assert_eq!(
        pedersen().B_blinding,
        RistrettoPoint::from_uniform_bytes(&uniform)
    );
    assert_eq!(
        pedersen().B,
        curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT
    );
    let keys = generate_keys(None);
    let other = generate_keys(None);
    assert_ne!(keys.ed25519_public_key, other.ed25519_public_key);
    assert_eq!(format!("{:?}", keys.secret.signing_secret), "Bytes(..)");
    assert_eq!(
        format!("{}", error(Some(1), "check")),
        "protocol-error: Some(1): check"
    );
}

#[test]
fn original_openings_remain_available_for_authorized_audits() {
    for f in [fixture(), count_only()] {
        for (i, state) in f.states.iter().enumerate() {
            let opening =
                Opening::read(&state.audit_opening, f.session.has_sum(), i as u32).unwrap();
            assert_eq!(opening.c, Scalar::from(2u64));
            if f.session.has_sum() {
                assert_eq!(opening.s, Scalar::from(10 + i as u64));
            }
            check_opening(
                &opening,
                &commitment_points(
                    &state.contribution.commitment,
                    f.session.has_sum(),
                    i as u32,
                )
                .unwrap(),
                f.session.has_sum(),
                i as u32,
                "audit-opening",
            )
            .unwrap();
            let encoded = serde_json::to_string(state).unwrap();
            let restored: ContributionState = serde_json::from_str(&encoded).unwrap();
            let opening =
                Opening::read(&restored.audit_opening, f.session.has_sum(), i as u32).unwrap();
            check_opening(
                &opening,
                &commitment_points(
                    &state.contribution.commitment,
                    f.session.has_sum(),
                    i as u32,
                )
                .unwrap(),
                f.session.has_sum(),
                i as u32,
                "audit-opening",
            )
            .unwrap();
        }
    }
    let value = serde_json::to_value(&count_only().output).unwrap();
    assert!(value["outcome"].get("sum").is_none());
    assert!(value["outcome"].get("sigma").is_none());
}

#[test]
fn signatures_and_encryption_bind_each_context_field() {
    let f = fixture();
    for rule in [false, true] {
        let mut c = f.contributions[1].clone();
        if rule {
            c.rule_digest.0[0] ^= 1;
        } else {
            c.session_id.0[0] ^= 1;
        }
        reject(
            verify_signature(
                &f.session.parameters.roster[1],
                &contribution_bytes(&c),
                &c.signature,
                1,
            ),
            Some(1),
            "signature",
        );
        let mut p = f.partials[1].local.clone();
        if rule {
            p.rule_digest.0[0] ^= 1;
        } else {
            p.session_id.0[0] ^= 1;
        }
        reject(
            verify_signature(
                &f.session.parameters.roster[1],
                &partial_bytes(&p),
                &p.signature,
                1,
            ),
            Some(1),
            "signature",
        );
        let mut o = f.output.clone();
        if rule {
            o.rule_digest.0[0] ^= 1;
        } else {
            o.session_id.0[0] ^= 1;
        }
        reject(
            verify_signature(
                &f.session.parameters.roster[0],
                &outcome_bytes(&o),
                &o.signature,
                0,
            ),
            Some(0),
            "signature",
        );
    }
    let plain = b"context binding";
    let digest = &f.output.contributions_digest;
    let packet = seal(&f.session, b"partial", 1, 0, digest, plain, &mut OsRng).unwrap();
    for field in [
        "session",
        "rules",
        "sender",
        "recipient",
        "purpose",
        "inputs",
    ] {
        let mut s = f.session.clone();
        let mut i = 1;
        let mut j = 0;
        let mut purpose = b"partial".as_slice();
        let mut digest = digest.clone();
        match field {
            "session" => s.parameters.session_id.0[0] ^= 1,
            "rules" => s.rule_digest.0[0] ^= 1,
            "sender" => i = 2,
            "recipient" => j = 2,
            "purpose" => purpose = b"share",
            "inputs" => digest.0[0] ^= 1,
            _ => unreachable!(),
        }
        reject(
            open(&s, purpose, i, j, &digest, &packet, &f.keys[0].secret),
            Some(i),
            "aead",
        );
    }
    assert_eq!(
        open(
            &f.session,
            b"partial",
            1,
            0,
            digest,
            &packet,
            &f.keys[0].secret
        )
        .unwrap(),
        plain
    );
    let mut cs = f.contributions.clone();
    cs[1].signature.0[0] ^= 1;
    assert_eq!(f.receive(&cs).err().unwrap().code, "input-not-attested");
    let mut cs = f.contributions.clone();
    cs.remove(1);
    assert_eq!(f.receive(&cs).err().unwrap().code, "input-not-attested");
    let mut cs = f.contributions.clone();
    cs.push(cs[1].clone());
    assert_eq!(f.receive(&cs).err().unwrap().code, "input-not-attested");
}

#[test]
fn signed_encodings_bind_the_claimed_index() {
    let f = fixture();
    let mut c = f.contributions[1].clone();
    c.index = 2;
    reject(
        verify_signature(
            &f.session.parameters.roster[1],
            &contribution_bytes(&c),
            &c.signature,
            1,
        ),
        Some(1),
        "signature",
    );
    let mut p = f.partials[1].local.clone();
    p.index = 2;
    reject(
        verify_signature(
            &f.session.parameters.roster[1],
            &partial_bytes(&p),
            &p.signature,
            1,
        ),
        Some(1),
        "signature",
    );
    let mut o = f.output.clone();
    o.finisher = 1;
    reject(
        verify_signature(
            &f.session.parameters.roster[0],
            &outcome_bytes(&o),
            &o.signature,
            0,
        ),
        Some(0),
        "signature",
    );
}

#[test]
fn recipient_rechecks_attestation_even_with_a_valid_finisher_signature() {
    let f = fixture();
    for attack in [
        "signature",
        "range-proof",
        "count-share-sum",
        "roster-index",
        "duplicate-contributor",
        "missing-contributor",
    ] {
        let mut cs = f.contributions.clone();
        let mut out = f.output.clone();
        let index = if attack == "roster-index" { 7 } else { 1 };
        match attack {
            "signature" => {
                cs[1].signature = sign(
                    &generate_keys(Some([88; 32])).secret,
                    &contribution_bytes(&cs[1]),
                )
                .unwrap()
            }
            "range-proof" => {
                cs[1].range_proof = refusal().contributions[1].range_proof.clone();
                f.resign_contribution(&mut cs[1]);
            }
            "count-share-sum" => {
                cs[1].share_commitments[0].count = pb(pedersen().B);
                f.resign_contribution(&mut cs[1]);
            }
            "roster-index" => cs[1].index = 7,
            "duplicate-contributor" => {
                cs.push(cs[1].clone());
                let mut total = Opening::zero();
                for i in [0, 1, 2, 1] {
                    total.add(&Opening::read(&f.states[i].audit_opening, true, i as u32).unwrap());
                }
                out.outcome = Outcome::Release {
                    count: 8,
                    rho: sb(total.r),
                    sum: Some("44".into()),
                    sigma: Some(sb(total.t)),
                };
            }
            "missing-contributor" => {
                cs.remove(1);
                out.outcome = Outcome::TooFewContributors {};
            }
            _ => unreachable!(),
        }
        cs.sort_by_key(|c| c.index);
        out.contributions_digest = contributions_digest(&cs.iter().collect::<Vec<_>>());
        f.resign_output(&mut out);
        let error =
            verify_outcome(&f.session, &cs, &out).expect_err("recipient accepted unattested input");
        assert_eq!(error.code, "input-not-attested");
        assert_eq!(error.index, Some(index));
        assert_eq!(error.check, attack);
    }
}

fn simulation_inputs(compute_only: bool, sums: bool) -> (Parameters, Vec<PartyInput>) {
    let f = fixture();
    let mut p = f.session.parameters.clone();
    p.statistic = if sums {
        Statistic::CountAndSum
    } else {
        Statistic::CountOnly
    };
    p.min_group = if compute_only { 4 } else { 6 };
    p.roster[2].compute_only = compute_only;
    let parties = f
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| PartyInput {
            keys: k.secret.clone(),
            input: Input {
                count: if compute_only && i == 2 { 0 } else { 2 },
                sum: sums.then(|| {
                    if compute_only && i == 2 {
                        "0".into()
                    } else {
                        (10 + i).to_string()
                    }
                }),
            },
        })
        .collect();
    (p, parties)
}
fn comparison(statistic: ThresholdStatistic, op: ThresholdOp, value: u128) -> Threshold {
    Threshold {
        statistic,
        op,
        value: value.to_string(),
    }
}
fn threshold_simulation() -> &'static Simulation {
    static F: OnceLock<Simulation> = OnceLock::new();
    F.get_or_init(|| {
        let (mut p, parties) = simulation_inputs(false, true);
        p.threshold = Some(comparison(
            ThresholdStatistic::Sum,
            ThresholdOp::GreaterOrEqual,
            33,
        ));
        simulate_session(p, &parties).unwrap()
    })
}
fn simulation_total(f: &Simulation) -> Opening {
    let mut total = Opening::zero();
    for p in &f.partials {
        total.add(&Opening::read(&p.local.shares, f.session.has_sum(), p.local.index).unwrap());
    }
    total
}
fn resign_simulation(_f: &Simulation, o: &mut SignedOutcome) {
    o.signature = sign(
        &fixture().keys[o.finisher as usize].secret,
        &outcome_bytes(o),
    )
    .unwrap();
}

#[test]
fn threshold_verdicts_and_boundaries_through_real_messages() {
    for sums in [false, true] {
        for stat in [ThresholdStatistic::Count, ThresholdStatistic::Sum] {
            if !sums && stat == ThresholdStatistic::Sum {
                continue;
            }
            let actual = if stat == ThresholdStatistic::Count {
                6
            } else {
                33
            };
            for op in [ThresholdOp::GreaterOrEqual, ThresholdOp::LessThan] {
                for value in [0, actual, actual + 1, u128::MAX] {
                    let (mut p, parties) = simulation_inputs(false, sums);
                    p.threshold = Some(comparison(stat, op, value));
                    let f = simulate_session(p, &parties).unwrap();
                    let expected = if op == ThresholdOp::GreaterOrEqual {
                        actual >= value
                    } else {
                        actual < value
                    };
                    assert!(
                        matches!(f.outcome.outcome, Outcome::ThresholdVerdict { verdict, .. } if verdict == expected)
                    );
                    let wire = serde_json::to_value(&f.outcome.outcome).unwrap();
                    assert_eq!(wire.as_object().unwrap().len(), 4);
                    for field in ["count", "sum", "rho", "sigma"] {
                        assert!(wire.get(field).is_none());
                    }
                    verify_outcome(&f.session, &f.contributions, &f.outcome).unwrap();
                }
            }
        }
    }
    let (mut p, mut parties) = simulation_inputs(false, true);
    p.max_value_bits = 64;
    p.threshold = Some(comparison(
        ThresholdStatistic::Sum,
        ThresholdOp::GreaterOrEqual,
        2 * u64::MAX as u128,
    ));
    for party in &mut parties {
        party.input.sum = Some(u64::MAX.to_string());
    }
    let f = simulate_session(p, &parties).unwrap();
    assert!(matches!(
        f.outcome.outcome,
        Outcome::ThresholdVerdict { verdict: true, .. }
    ));
}

#[test]
fn compute_only_and_requester_participation() {
    for sums in [false, true] {
        let (p, parties) = simulation_inputs(true, sums);
        let f = simulate_session(p, &parties).unwrap();
        assert!(
            matches!(&f.outcome.outcome, Outcome::Release { count: 4, sum, .. } if *sum == sums.then(|| "21".into()))
        );
        // The requester uses the third roster identity, participates in both rounds and finishes.
        let messages = f
            .partials
            .iter()
            .flat_map(|p| p.messages.iter().filter(|m| m.recipient == 2).cloned())
            .collect::<Vec<_>>();
        let outcome = finish(
            &f.session,
            2,
            &parties[2].keys,
            &f.contributions,
            Some(&f.partials[2].local),
            &messages,
        )
        .unwrap();
        assert_eq!(outcome.finisher, 2);
        verify_outcome(&f.session, &f.contributions, &outcome).unwrap();
        let mut bad = parties[2].input.clone();
        bad.count = 1;
        reject(
            contribute(&f.session, 2, &parties[2].keys, &bad),
            Some(2),
            "compute-only-input",
        );
        if sums {
            bad.count = 0;
            bad.sum = Some("1".into());
            reject(
                contribute(&f.session, 2, &parties[2].keys, &bad),
                Some(2),
                "compute-only-input",
            );
        }
        // Keep share sums valid: only the compute-only range constraint rejects these inputs.
        let invalid = if sums {
            vec![
                (1, 1, u32::MAX as u64, max_value(48)),
                (1, 0, u32::MAX as u64, 0),
                (0, 1, 0, max_value(48)),
            ]
        } else {
            vec![(1, 0, u32::MAX as u64, 0)]
        };
        for (count, sum, count_bound, sum_bound) in invalid {
            let mut cs = f.contributions.clone();
            let c = &mut cs[2];
            let (r, t) = (Scalar::random(&mut OsRng), Scalar::random(&mut OsRng));
            c.commitment = Opening {
                c: Scalar::from(count),
                r,
                s: Scalar::from(sum),
                t,
            }
            .commitments(sums);
            let (values, blinds, bits) = if sums {
                (
                    vec![count, count_bound - count, sum, sum_bound - sum],
                    vec![r, -r, t, -t],
                    64,
                )
            } else {
                // Match the zero proof's arity so only the value constraint rejects it.
                (vec![count, count_bound - count], vec![r, -r], 32)
            };
            c.range_proof = prove(
                &f.session,
                b"input-range",
                2,
                &values,
                &blinds,
                bits,
                &mut OsRng,
            )
            .unwrap();
            c.share_commitments = vec![
                c.commitment.clone(),
                Opening::zero().commitments(sums),
                Opening::zero().commitments(sums),
            ];
            c.signature = sign(&parties[2].keys, &contribution_bytes(c)).unwrap();
            reject(
                verify_outcome(&f.session, &cs, &outcome),
                Some(2),
                "range-proof",
            );
        }
    }
    let (mut p, parties) = simulation_inputs(true, true);
    p.threshold = Some(comparison(
        ThresholdStatistic::Sum,
        ThresholdOp::GreaterOrEqual,
        21,
    ));
    let f = simulate_session(p, &parties).unwrap();
    assert!(matches!(
        f.outcome.outcome,
        Outcome::ThresholdVerdict { verdict: true, .. }
    ));
}

#[test]
fn threshold_parameters_and_compute_role_are_bound() {
    let f = threshold_simulation();
    for field in ["statistic", "op", "value", "absent", "computeOnly"] {
        let mut p = f.session.parameters.clone();
        match field {
            "statistic" => p.threshold.as_mut().unwrap().statistic = ThresholdStatistic::Count,
            "op" => p.threshold.as_mut().unwrap().op = ThresholdOp::LessThan,
            "value" => p.threshold.as_mut().unwrap().value = "34".into(),
            "absent" => p.threshold = None,
            "computeOnly" => p.roster[2].compute_only = true,
            _ => unreachable!(),
        }
        let s = Session::new(p).unwrap();
        assert_ne!(s.rule_digest, f.session.rule_digest, "unbound {field}");
        reject(
            verify_outcome(&s, &f.contributions, &f.outcome),
            Some(0),
            "rule-digest",
        );
    }
    for value in ["-1", "01", "+1", "340282366920938463463374607431768211456"] {
        let mut p = f.session.parameters.clone();
        p.threshold.as_mut().unwrap().value = value.into();
        reject(Session::new(p), None, "threshold-value");
    }
    let mut p = f.session.parameters.clone();
    p.statistic = Statistic::CountOnly;
    reject(Session::new(p), None, "threshold-statistic");
}

#[test]
fn threshold_verifier_rejects_forged_evidence() {
    let f = threshold_simulation();
    for check in [
        "signature",
        "threshold-commitment",
        "threshold-limbs",
        "point-encoding",
        "range-proof",
    ] {
        let mut o = f.outcome.clone();
        if let Outcome::ThresholdVerdict {
            verdict,
            limb_commitments,
            range_proof,
        } = &mut o.outcome
        {
            match check {
                "signature" | "threshold-commitment" => *verdict = !*verdict,
                "threshold-limbs" => limb_commitments.push(pb(pedersen().B)),
                "point-encoding" => limb_commitments[0] = Bytes(vec![255; 32]),
                "range-proof" => range_proof.0[0] ^= 1,
                _ => unreachable!(),
            }
        }
        if check != "signature" {
            resign_simulation(f, &mut o);
        }
        reject(
            verify_outcome(&f.session, &f.contributions, &o),
            Some(0),
            check,
        );
    }
    let mut o = f.outcome.clone();
    let total = simulation_total(f);
    o.outcome = Outcome::Release {
        count: 6,
        rho: sb(total.r),
        sum: Some("33".into()),
        sigma: Some(sb(total.t)),
    };
    resign_simulation(f, &mut o);
    reject(
        verify_outcome(&f.session, &f.contributions, &o),
        Some(0),
        "threshold-required",
    );
    let mut o = fixture().output.clone();
    o.outcome = f.outcome.outcome.clone();
    fixture().resign_output(&mut o);
    reject(fixture().verify(&o), Some(0), "threshold-disabled");
    // All fields of the threshold evidence must be signed, including a proof's bytes.
    for field in ["verdict", "limb", "proof"] {
        let mut o = f.outcome.clone();
        if let Outcome::ThresholdVerdict {
            verdict,
            limb_commitments,
            range_proof,
        } = &mut o.outcome
        {
            match field {
                "verdict" => *verdict = !*verdict,
                "limb" => limb_commitments[0] = pb(pedersen().B),
                "proof" => range_proof.0[0] ^= 1,
                _ => unreachable!(),
            }
        }
        reject(
            verify_outcome(&f.session, &f.contributions, &o),
            Some(0),
            "signature",
        );
    }
}

// Construct independent valid limb range proofs, optionally for a commitment to a
// negative difference. Re-signing ensures the mathematical verifier must reject it.
fn comparison_evidence(s: &Session, gaps: &[(Scalar, Scalar)], values: &[u64]) -> Outcome {
    let mut limbs = Vec::new();
    let mut blindings = Vec::new();
    for (gap, blind) in gaps {
        let low = Scalar::random(&mut OsRng);
        let high = (*blind - low) * limb_weight().invert();
        limbs.extend([pb(commit(*gap, low)), pb(commit(Scalar::ZERO, high))]);
        blindings.extend([low, high]);
    }
    Outcome::ThresholdVerdict {
        verdict: true,
        limb_commitments: limbs,
        range_proof: prove(
            s,
            b"threshold-verdict",
            0,
            values,
            &blindings,
            64,
            &mut OsRng,
        )
        .unwrap(),
    }
}

#[test]
fn threshold_requires_a_range_proof_and_minimum_group_evidence() {
    let f = threshold_simulation();
    let total = simulation_total(f);
    let mut o = f.outcome.clone();
    // A valid range proof about an unrelated minimum cannot establish the group rule.
    o.outcome = comparison_evidence(
        &f.session,
        &[(Scalar::ZERO, total.t), (Scalar::ONE, total.r)],
        &[0, 0, 1, 0],
    );
    resign_simulation(f, &mut o);
    reject(
        verify_outcome(&f.session, &f.contributions, &o),
        Some(0),
        "threshold-minimum",
    );
    // The limb relation is correct for a false verdict, but its negative witness is out of range.
    o.outcome = comparison_evidence(
        &f.session,
        &[(-Scalar::ONE, -total.t), (Scalar::ZERO, total.r)],
        &[0, 0, 0, 0],
    );
    if let Outcome::ThresholdVerdict { verdict, .. } = &mut o.outcome {
        *verdict = false;
    }
    resign_simulation(f, &mut o);
    reject(
        verify_outcome(&f.session, &f.contributions, &o),
        Some(0),
        "range-proof",
    );
    let (mut p, parties) = simulation_inputs(false, true);
    p.min_group = 7;
    p.threshold = f.session.parameters.threshold.clone();
    let below = simulate_session(p, &parties).unwrap();
    assert!(matches!(
        below.outcome.outcome,
        Outcome::BelowMinimumGroup { .. }
    ));
    let total = simulation_total(&below);
    let mut o = below.outcome.clone();
    o.outcome = comparison_evidence(
        &below.session,
        &[(Scalar::ZERO, total.t), (Scalar::ZERO, total.r)],
        &[0, 0, 0, 0],
    );
    resign_simulation(&below, &mut o);
    reject(
        verify_outcome(&below.session, &below.contributions, &o),
        Some(0),
        "threshold-minimum",
    );
    // With the minimum disabled only the comparison limbs are needed.
    let (mut p, parties) = simulation_inputs(false, false);
    p.min_group = 0;
    p.threshold = Some(comparison(
        ThresholdStatistic::Count,
        ThresholdOp::LessThan,
        7,
    ));
    let f = simulate_session(p, &parties).unwrap();
    assert!(
        matches!(f.outcome.outcome, Outcome::ThresholdVerdict { verdict: true, ref limb_commitments, .. } if limb_commitments.len() == 2)
    );
}

#[test]
fn simulation_checks_inputs_and_too_few_attestations() {
    let (p, mut parties) = simulation_inputs(false, true);
    reject(
        simulate_session(p.clone(), &parties[..2]),
        None,
        "simulation-party-count",
    );
    parties[1].keys = generate_keys(Some([55; 32])).secret;
    reject(
        simulate_session(p, &parties),
        Some(1),
        "signing-key-mismatch",
    );
    let (mut p, mut parties) = simulation_inputs(false, true);
    p.roster.pop();
    parties.pop();
    p.threshold = Some(comparison(
        ThresholdStatistic::Sum,
        ThresholdOp::GreaterOrEqual,
        0,
    ));
    let f = simulate_session(p, &parties).unwrap();
    assert!(f.partials.is_empty());
    assert!(matches!(f.outcome.outcome, Outcome::TooFewContributors {}));
}

#[test]
fn roster_rejects_alternate_encodings_of_encryption_keys() {
    let f = fixture();
    let mut p = f.session.parameters.clone();
    p.roster[1].x25519_public_key = p.roster[0].x25519_public_key.clone();
    p.roster[1].x25519_public_key.0[31] |= 128;
    // The alias gives exactly the same shared secret as the first roster coordinate.
    let secret = StaticSecret::from([77; 32]);
    let original =
        PublicKey::from(fixed::<32>(&p.roster[0].x25519_public_key, None, "key").unwrap());
    let alias = PublicKey::from(fixed::<32>(&p.roster[1].x25519_public_key, None, "key").unwrap());
    assert_eq!(
        secret.diffie_hellman(&original).as_bytes(),
        secret.diffie_hellman(&alias).as_bytes()
    );
    reject(Session::new(p), Some(1), "invalid-encryption-key");
}

#[test]
fn finisher_rechecks_inputs_even_with_coherent_signed_partials() {
    let f = fixture();
    for check in ["signature", "range-proof"] {
        let mut cs = f.contributions.clone();
        if check == "signature" {
            cs[1].signature = sign(
                &generate_keys(Some([90; 32])).secret,
                &contribution_bytes(&cs[1]),
            )
            .unwrap();
        } else {
            cs[1].range_proof.0[0] ^= 1;
            f.resign_contribution(&mut cs[1]);
        }
        let digest = contributions_digest(&cs.iter().collect::<Vec<_>>());
        let mut ps = f
            .partials
            .iter()
            .map(|p| p.local.clone())
            .collect::<Vec<_>>();
        for p in &mut ps {
            p.contributions_digest = digest.clone();
            p.signature = sign(&f.keys[p.index as usize].secret, &partial_bytes(p)).unwrap();
        }
        let messages = ps[1..]
            .iter()
            .map(|p| PartialMessage {
                session_id: f.session.parameters.session_id.clone(),
                rule_digest: f.session.rule_digest.clone(),
                contributions_digest: digest.clone(),
                index: p.index,
                recipient: 0,
                packet: seal(
                    &f.session,
                    b"partial",
                    p.index,
                    0,
                    &digest,
                    &serde_json::to_vec(p).unwrap(),
                    &mut OsRng,
                )
                .unwrap(),
            })
            .collect::<Vec<_>>();
        let e = finish(
            &f.session,
            0,
            &f.keys[0].secret,
            &cs,
            Some(&ps[0]),
            &messages,
        )
        .unwrap_err();
        assert_eq!(e.code, "input-not-attested");
        assert_eq!(e.index, Some(1));
        assert_eq!(e.check, check);
    }
    let f = too_few();
    for missing in [false, true] {
        let mut cs = f.contributions.clone();
        if missing {
            cs.pop();
        } else {
            cs[1].signature.0[0] ^= 1;
        }
        let e = finish(&f.session, 0, &f.keys[0].secret, &cs, None, &[]).unwrap_err();
        assert_eq!(e.code, "input-not-attested");
        assert_eq!(e.index, Some(1));
        assert_eq!(
            e.check,
            if missing {
                "missing-contributor"
            } else {
                "signature"
            }
        );
    }
}

#[test]
fn all_compute_only_parties_compare_zero_totals() {
    for op in [ThresholdOp::GreaterOrEqual, ThresholdOp::LessThan] {
        let (mut p, mut parties) = simulation_inputs(true, true);
        p.min_group = 0;
        p.threshold = Some(comparison(ThresholdStatistic::Sum, op, 0));
        for party in &mut p.roster {
            party.compute_only = true;
        }
        for party in &mut parties {
            party.input = Input {
                count: 0,
                sum: Some("0".into()),
            };
        }
        let f = simulate_session(p, &parties).unwrap();
        verify_outcome(&f.session, &f.contributions, &f.outcome).unwrap();
        assert!(
            matches!(f.outcome.outcome, Outcome::ThresholdVerdict { verdict, .. } if verdict == (op == ThresholdOp::GreaterOrEqual))
        );
    }
}
