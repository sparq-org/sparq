//! Research-grade verifiable joint aggregation. Not externally audited.
//! Callers agree an authenticated roster and carry messages between parties.
mod crypto;
mod wire;
pub use crypto::generate_keys;
pub use wire::*;

use crypto::*;
use curve25519_dalek::{ristretto::RistrettoPoint, scalar::Scalar, traits::Identity};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand_core::{CryptoRng, OsRng, RngCore};
use std::collections::HashSet;
use x25519_dalek::{PublicKey, StaticSecret};

impl Session {
    /// Validate parameters and bind the ordered roster and rules to this session.
    pub fn new(parameters: Parameters) -> Result<Self> {
        validate_parameters(&parameters)?;
        let rule_digest = rule_digest(&parameters);
        Ok(Self {
            parameters,
            rule_digest,
        })
    }
    fn validate(&self) -> Result<()> {
        validate_parameters(&self.parameters)?;
        guard(
            self.rule_digest == rule_digest(&self.parameters),
            None,
            "rule-digest",
        )
    }
    fn party(&self, i: u32) -> Result<&Party> {
        self.parameters
            .roster
            .get(i as usize)
            .ok_or_else(|| error(Some(i), "roster-index"))
    }
    fn identity(&self, i: u32, keys: &SecretKeys) -> Result<()> {
        let party = self.party(i)?;
        let signing = SigningKey::from_bytes(&fixed(&keys.signing_secret, Some(i), "signing-key")?);
        let encryption =
            StaticSecret::from(fixed(&keys.encryption_secret, Some(i), "encryption-key")?);
        guard(
            signing.verifying_key().as_bytes().as_slice() == party.ed25519_public_key.0,
            Some(i),
            "signing-key-mismatch",
        )?;
        guard(
            PublicKey::from(&encryption).as_bytes().as_slice() == party.x25519_public_key.0,
            Some(i),
            "encryption-key-mismatch",
        )
    }
    fn has_sum(&self) -> bool {
        self.parameters.statistic == Statistic::CountAndSum
    }
    fn input_bounds(&self, i: u32) -> Result<(u64, u64)> {
        Ok(if self.party(i)?.compute_only {
            (0, 0)
        } else {
            (u32::MAX as u64, max_value(self.parameters.max_value_bits))
        })
    }
    fn context(&self, sid: &Bytes, rule: &Bytes, i: u32) -> Result<()> {
        guard(*sid == self.parameters.session_id, Some(i), "session-id")?;
        guard(*rule == self.rule_digest, Some(i), "rule-digest")
    }
}
fn validate_parameters(p: &Parameters) -> Result<()> {
    fixed::<32>(&p.session_id, None, "session-id")?;
    guard(
        !p.roster.is_empty() && p.roster.len() <= 1024,
        None,
        "roster-size",
    )?;
    guard(p.min_contributors > 0, None, "minimum-contributors")?;
    guard((1..=64).contains(&p.max_value_bits), None, "value-bits")?;
    if let Some(t) = &p.threshold {
        guard(
            t.statistic != ThresholdStatistic::Sum || p.statistic == Statistic::CountAndSum,
            None,
            "threshold-statistic",
        )?;
        let v = t
            .value
            .parse::<u128>()
            .map_err(|_| error(None, "threshold-value"))?;
        guard(v.to_string() == t.value, None, "threshold-value")?;
    }
    let mut iris = HashSet::new();
    let mut signing = HashSet::new();
    let mut encryption = HashSet::new();
    for (i, party) in p.roster.iter().enumerate() {
        let i = i as u32;
        guard(
            !party.party_iri.is_empty() && iris.insert(&party.party_iri),
            Some(i),
            "duplicate-identity",
        )?;
        let key = fixed(&party.ed25519_public_key, Some(i), "signing-key")?;
        let key = VerifyingKey::from_bytes(&key).map_err(|_| error(Some(i), "signing-key"))?;
        guard(
            !key.is_weak() && signing.insert(party.ed25519_public_key.0.clone()),
            Some(i),
            "duplicate-signing-key",
        )?;
        let encoded: [u8; 32] = fixed(&party.x25519_public_key, Some(i), "encryption-key")?;
        let mut modulus = [255; 32];
        modulus[0] = 237;
        modulus[31] = 127;
        // X25519 decoding masks/reduces alternate encodings. Require canonical roster
        // keys so byte-wise uniqueness also implies distinct encryption coordinates.
        guard(
            encoded.iter().rev().cmp(modulus.iter().rev()).is_lt(),
            Some(i),
            "invalid-encryption-key",
        )?;
        let key = PublicKey::from(encoded);
        let shared = StaticSecret::from([0x42; 32]).diffie_hellman(&key);
        guard(
            shared.was_contributory() && encryption.insert(party.x25519_public_key.0.clone()),
            Some(i),
            "invalid-encryption-key",
        )?;
    }
    Ok(())
}
fn max_value(bits: u8) -> u64 {
    if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}
fn decimal(text: &str, i: u32) -> Result<u128> {
    let v = text
        .parse::<u128>()
        .map_err(|_| error(Some(i), "integer-encoding"))?;
    guard(v.to_string() == text, Some(i), "integer-encoding")?;
    Ok(v)
}
#[derive(Clone)]
struct Opening {
    c: Scalar,
    r: Scalar,
    s: Scalar,
    t: Scalar,
}
impl Opening {
    fn zero() -> Self {
        Self {
            c: Scalar::ZERO,
            r: Scalar::ZERO,
            s: Scalar::ZERO,
            t: Scalar::ZERO,
        }
    }
    fn wire(&self, sums: bool) -> Shares {
        Shares {
            count: sb(self.c),
            rho: sb(self.r),
            sum: sums.then(|| sb(self.s)),
            sigma: sums.then(|| sb(self.t)),
        }
    }
    fn read(w: &Shares, sums: bool, i: u32) -> Result<Self> {
        guard(
            w.sum.is_some() == sums && w.sigma.is_some() == sums,
            Some(i),
            "statistic-shape",
        )?;
        Ok(Self {
            c: scalar(&w.count, i)?,
            r: scalar(&w.rho, i)?,
            s: w.sum
                .as_ref()
                .map(|v| scalar(v, i))
                .transpose()?
                .unwrap_or(Scalar::ZERO),
            t: w.sigma
                .as_ref()
                .map(|v| scalar(v, i))
                .transpose()?
                .unwrap_or(Scalar::ZERO),
        })
    }
    fn commitments(&self, sums: bool) -> Commitments {
        Commitments {
            count: pb(commit(self.c, self.r)),
            sum: sums.then(|| pb(commit(self.s, self.t))),
        }
    }
    fn add(&mut self, v: &Self) {
        self.c += v.c;
        self.r += v.r;
        self.s += v.s;
        self.t += v.t;
    }
    fn sub(&mut self, v: &Self) {
        self.c -= v.c;
        self.r -= v.r;
        self.s -= v.s;
        self.t -= v.t;
    }
    fn random(sums: bool, rng: &mut (impl RngCore + CryptoRng)) -> Self {
        Self {
            c: Scalar::random(&mut *rng),
            r: Scalar::random(&mut *rng),
            s: if sums {
                Scalar::random(&mut *rng)
            } else {
                Scalar::ZERO
            },
            t: if sums {
                Scalar::random(rng)
            } else {
                Scalar::ZERO
            },
        }
    }
}
fn split(
    value: &Opening,
    n: usize,
    sums: bool,
    rng: &mut (impl RngCore + CryptoRng),
) -> Vec<Opening> {
    let mut last = value.clone();
    let mut parts = Vec::with_capacity(n);
    for _ in 1..n {
        let part = Opening::random(sums, rng);
        last.sub(&part);
        parts.push(part);
    }
    parts.push(last);
    parts
}

/// Produce a signed public contribution, a private local share and a private audit opening.
pub fn contribute(
    s: &Session,
    index: u32,
    keys: &SecretKeys,
    input: &Input,
) -> Result<ContributionState> {
    contribute_rng(s, index, keys, input, &mut OsRng)
}
fn contribute_rng(
    s: &Session,
    i: u32,
    keys: &SecretKeys,
    input: &Input,
    rng: &mut (impl RngCore + CryptoRng),
) -> Result<ContributionState> {
    s.validate()?;
    s.identity(i, keys)?;
    guard(input.count <= u32::MAX as u64, Some(i), "input-range")?;
    guard(
        input.sum.is_some() == s.has_sum(),
        Some(i),
        "statistic-shape",
    )?;
    let sum = input
        .sum
        .as_ref()
        .map(|v| decimal(v, i))
        .transpose()?
        .unwrap_or(0);
    guard(
        !s.party(i)?.compute_only || (input.count == 0 && sum == 0),
        Some(i),
        "compute-only-input",
    )?;
    guard(
        sum <= max_value(s.parameters.max_value_bits) as u128,
        Some(i),
        "input-range",
    )?;
    let value = Opening {
        c: Scalar::from(input.count),
        r: Scalar::random(&mut *rng),
        s: Scalar::from(sum as u64),
        t: if s.has_sum() {
            Scalar::random(&mut *rng)
        } else {
            Scalar::ZERO
        },
    };
    let (count_bound, sum_bound) = s.input_bounds(i)?;
    let (values, blindings, bits) = if s.has_sum() {
        // A common width plus complementary values enforces both requested bounds.
        (
            vec![
                input.count,
                count_bound - input.count,
                sum as u64,
                sum_bound - sum as u64,
            ],
            vec![value.r, -value.r, value.t, -value.t],
            if s.parameters.max_value_bits <= 32 {
                32
            } else {
                64
            },
        )
    } else if s.party(i)?.compute_only {
        (vec![0, 0], vec![value.r, -value.r], 32)
    } else {
        (vec![input.count], vec![value.r], 32)
    };
    let range_proof = prove(s, b"input-range", i, &values, &blindings, bits, rng)?;
    let parts = split(&value, s.parameters.roster.len(), s.has_sum(), rng);
    let mut ciphertexts = Vec::with_capacity(parts.len());
    for (j, part) in parts.iter().enumerate() {
        ciphertexts.push(if j == i as usize {
            None
        } else {
            Some(seal(
                s,
                b"share",
                i,
                j as u32,
                &Bytes::default(),
                &serde_json::to_vec(&part.wire(s.has_sum()))
                    .map_err(|_| error(Some(i), "share-encoding"))?,
                rng,
            )?)
        });
    }
    let mut c = Contribution {
        session_id: s.parameters.session_id.clone(),
        rule_digest: s.rule_digest.clone(),
        index: i,
        commitment: value.commitments(s.has_sum()),
        range_proof,
        share_commitments: parts.iter().map(|p| p.commitments(s.has_sum())).collect(),
        ciphertexts,
        signature: Bytes::default(),
    };
    c.signature = sign(keys, &contribution_bytes(&c))?;
    Ok(ContributionState {
        audit_opening: value.wire(s.has_sum()),
        contribution: c,
        local: LocalShare {
            session_id: s.parameters.session_id.clone(),
            rule_digest: s.rule_digest.clone(),
            index: i,
            shares: parts[i as usize].wire(s.has_sum()),
        },
    })
}
fn commitment_points(
    c: &Commitments,
    sums: bool,
    i: u32,
) -> Result<(RistrettoPoint, RistrettoPoint)> {
    guard(c.sum.is_some() == sums, Some(i), "statistic-shape")?;
    Ok((
        point(&c.count, i)?,
        c.sum
            .as_ref()
            .map(|v| point(v, i))
            .transpose()?
            .unwrap_or(RistrettoPoint::identity()),
    ))
}
fn verify_contribution(s: &Session, c: &Contribution) -> Result<()> {
    let i = c.index;
    let party = s.party(i)?;
    s.context(&c.session_id, &c.rule_digest, i)?;
    verify_signature(party, &contribution_bytes(c), &c.signature, i)?;
    let n = s.parameters.roster.len();
    guard(
        c.share_commitments.len() == n && c.ciphertexts.len() == n,
        Some(i),
        "share-count",
    )?;
    for (j, packet) in c.ciphertexts.iter().enumerate() {
        guard(
            packet.is_some() == (i as usize != j),
            Some(i),
            "ciphertext-layout",
        )?;
    }
    let (count, sum) = commitment_points(&c.commitment, s.has_sum(), i)?;
    let (count_bound, sum_bound) = s.input_bounds(i)?;
    let (points, bits) = if s.has_sum() {
        (
            vec![
                count,
                Scalar::from(count_bound) * pedersen().B - count,
                sum,
                Scalar::from(sum_bound) * pedersen().B - sum,
            ],
            if s.parameters.max_value_bits <= 32 {
                32
            } else {
                64
            },
        )
    } else if party.compute_only {
        (vec![count, -count], 32)
    } else {
        (vec![count], 32)
    };
    verify_proof(s, b"input-range", i, &c.range_proof, &points, bits)?;
    let (mut cc, mut dd) = (RistrettoPoint::identity(), RistrettoPoint::identity());
    for share in &c.share_commitments {
        let (a, b) = commitment_points(share, s.has_sum(), i)?;
        cc += a;
        dd += b;
    }
    guard(cc == count, Some(i), "count-share-sum")?;
    guard(dd == sum, Some(i), "sum-share-sum")
}
fn verified_contributions<'a>(
    s: &Session,
    cs: &'a [Contribution],
) -> Result<Vec<&'a Contribution>> {
    s.validate()?;
    let mut ordered = vec![None; s.parameters.roster.len()];
    for c in cs {
        let result = (|| {
            s.party(c.index)?;
            guard(
                ordered[c.index as usize].is_none(),
                Some(c.index),
                "duplicate-contributor",
            )?;
            verify_contribution(s, c)?;
            ordered[c.index as usize] = Some(c);
            Ok(())
        })();
        result.map_err(|mut e: Error| {
            e.code = "input-not-attested".into();
            e
        })?;
    }
    ordered
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            c.ok_or_else(|| Error {
                code: "input-not-attested".into(),
                index: Some(i as u32),
                check: "missing-contributor".into(),
            })
        })
        .collect()
}
fn check_opening(
    v: &Opening,
    c: &(RistrettoPoint, RistrettoPoint),
    sums: bool,
    i: u32,
    check: &str,
) -> Result<()> {
    guard(
        commit(v.c, v.r) == c.0 && (!sums || commit(v.s, v.t) == c.1),
        Some(i),
        check,
    )
}
fn partial_commitments(
    cs: &[&Contribution],
    j: usize,
    sums: bool,
) -> Result<(RistrettoPoint, RistrettoPoint)> {
    let (mut c, mut d) = (RistrettoPoint::identity(), RistrettoPoint::identity());
    for contribution in cs {
        let (a, b) =
            commitment_points(&contribution.share_commitments[j], sums, contribution.index)?;
        c += a;
        d += b;
    }
    Ok((c, d))
}

/// Verify all inputs, decrypt this party's shares, and encrypt its signed partial to peers.
/// Call once for the agreed contribution set; all returned state is contributor-private.
pub fn receive_and_partial(
    s: &Session,
    index: u32,
    keys: &SecretKeys,
    cs: &[Contribution],
    local: &LocalShare,
) -> Result<PartialDispatch> {
    let cs = verified_contributions(s, cs)?;
    s.identity(index, keys)?;
    guard(
        cs.len() >= s.parameters.min_contributors as usize,
        Some(index),
        "too-few-contributors",
    )?;
    guard(local.index == index, Some(index), "local-index")?;
    s.context(&local.session_id, &local.rule_digest, index)?;
    let mut total = Opening::zero();
    for c in &cs {
        let shares = if c.index == index {
            local.shares.clone()
        } else {
            let packet = c.ciphertexts[index as usize]
                .as_ref()
                .ok_or_else(|| error(Some(c.index), "ciphertext-layout"))?;
            serde_json::from_slice::<Shares>(&open(
                s,
                b"share",
                c.index,
                index,
                &Bytes::default(),
                packet,
                keys,
            )?)
            .map_err(|_| error(Some(c.index), "share-encoding"))?
        };
        let value = Opening::read(&shares, s.has_sum(), c.index)?;
        check_opening(
            &value,
            &commitment_points(&c.share_commitments[index as usize], s.has_sum(), c.index)?,
            s.has_sum(),
            c.index,
            "share-commitment",
        )?;
        total.add(&value);
    }
    let digest = contributions_digest(&cs);
    let mut partial = Partial {
        session_id: s.parameters.session_id.clone(),
        rule_digest: s.rule_digest.clone(),
        contributions_digest: digest.clone(),
        index,
        shares: total.wire(s.has_sum()),
        signature: Bytes::default(),
    };
    partial.signature = sign(keys, &partial_bytes(&partial))?;
    let plain = serde_json::to_vec(&partial).map_err(|_| error(Some(index), "partial-encoding"))?;
    let mut messages = Vec::new();
    for j in 0..cs.len() as u32 {
        if j != index {
            messages.push(PartialMessage {
                session_id: s.parameters.session_id.clone(),
                rule_digest: s.rule_digest.clone(),
                contributions_digest: digest.clone(),
                index,
                recipient: j,
                packet: seal(s, b"partial", index, j, &digest, &plain, &mut OsRng)?,
            });
        }
    }
    Ok(PartialDispatch {
        local: partial,
        messages,
    })
}
fn check_partial(
    s: &Session,
    cs: &[&Contribution],
    digest: &Bytes,
    p: &Partial,
) -> Result<Opening> {
    s.party(p.index)?;
    s.context(&p.session_id, &p.rule_digest, p.index)?;
    guard(
        p.contributions_digest == *digest,
        Some(p.index),
        "contributions-digest",
    )?;
    verify_signature(s.party(p.index)?, &partial_bytes(p), &p.signature, p.index)?;
    let v = Opening::read(&p.shares, s.has_sum(), p.index)?;
    check_opening(
        &v,
        &partial_commitments(cs, p.index as usize, s.has_sum())?,
        s.has_sum(),
        p.index,
        "partial-commitment",
    )?;
    Ok(v)
}
fn scalar_integer(s: Scalar, max: u128, i: u32) -> Result<u128> {
    let bytes = s.to_bytes();
    guard(
        bytes[16..].iter().all(|b| *b == 0),
        Some(i),
        "aggregate-range",
    )?;
    let v = u128::from_le_bytes(
        bytes[..16]
            .try_into()
            .map_err(|_| error(Some(i), "aggregate-range"))?,
    );
    guard(v <= max, Some(i), "aggregate-range")?;
    Ok(v)
}

fn integer_scalar(v: u128) -> Scalar {
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(&v.to_le_bytes());
    Scalar::from_bytes_mod_order(bytes)
}
fn limb_weight() -> Scalar {
    integer_scalar(1u128 << 64)
}
fn threshold_ge(t: &Threshold, verdict: bool) -> bool {
    (t.op == ThresholdOp::GreaterOrEqual) == verdict
}
fn threshold_outcome(s: &Session, t: &Threshold, total: &Opening, i: u32) -> Result<Outcome> {
    let (value, blinding, max) = match t.statistic {
        ThresholdStatistic::Count => (total.c, total.r, u32::MAX as u128),
        ThresholdStatistic::Sum => (
            total.s,
            total.t,
            max_value(s.parameters.max_value_bits) as u128,
        ),
    };
    let value = scalar_integer(value, s.parameters.roster.len() as u128 * max, i)?;
    let threshold = decimal(&t.value, i)?;
    let ge = value >= threshold;
    let verdict = if t.op == ThresholdOp::GreaterOrEqual {
        ge
    } else {
        !ge
    };
    let (difference, blind) = if ge {
        (value - threshold, blinding)
    } else {
        (threshold - 1 - value, -blinding)
    };
    let mut differences = vec![(difference, blind)];
    if s.parameters.min_group > 0 {
        let count = scalar_integer(
            total.c,
            s.parameters.roster.len() as u128 * u32::MAX as u128,
            i,
        )?;
        let gap = count
            .checked_sub(s.parameters.min_group as u128)
            .ok_or_else(|| error(Some(i), "minimum-group"))?;
        differences.push((gap, total.r));
    }
    let mut values = Vec::new();
    let mut blindings = Vec::new();
    let mut limb_commitments = Vec::new();
    for (gap, blind) in differences {
        // A pair of hidden limbs supports totals beyond the library's single-value width.
        // Randomize both limb commitments; neither publishes a piece of the total.
        let low_blind = Scalar::random(&mut OsRng);
        let high_blind = (blind - low_blind) * limb_weight().invert();
        for (v, r) in [(gap as u64, low_blind), ((gap >> 64) as u64, high_blind)] {
            values.push(v);
            blindings.push(r);
            limb_commitments.push(pb(commit(Scalar::from(v), r)));
        }
    }
    Ok(Outcome::ThresholdVerdict {
        verdict,
        limb_commitments,
        range_proof: prove(
            s,
            b"threshold-verdict",
            i,
            &values,
            &blindings,
            64,
            &mut OsRng,
        )?,
    })
}

fn verify_threshold(
    s: &Session,
    i: u32,
    verdict: bool,
    limbs: &[Bytes],
    proof: &Bytes,
    aggregates: (RistrettoPoint, RistrettoPoint),
) -> Result<()> {
    let t = s
        .parameters
        .threshold
        .as_ref()
        .ok_or_else(|| error(Some(i), "threshold-disabled"))?;
    let minimum = s.parameters.min_group;
    guard(
        limbs.len() == if minimum > 0 { 4 } else { 2 },
        Some(i),
        "threshold-limbs",
    )?;
    let points = limbs
        .iter()
        .map(|p| point(p, i))
        .collect::<Result<Vec<_>>>()?;
    let aggregate = match t.statistic {
        ThresholdStatistic::Count => aggregates.0,
        ThresholdStatistic::Sum => aggregates.1,
    };
    let target = integer_scalar(decimal(&t.value, i)?) * pedersen().B;
    let difference = if threshold_ge(t, verdict) {
        aggregate - target
    } else {
        target - pedersen().B - aggregate
    };
    guard(
        points[0] + limb_weight() * points[1] == difference,
        Some(i),
        "threshold-commitment",
    )?;
    if minimum > 0 {
        guard(
            points[2] + limb_weight() * points[3]
                == aggregates.0 - Scalar::from(minimum) * pedersen().B,
            Some(i),
            "threshold-minimum",
        )?;
    }
    // Each difference is below the scalar modulus. A negative difference modulo that
    // modulus cannot have this unsigned limb representation, so the comparison is strict.
    verify_proof(s, b"threshold-verdict", i, proof, &points, 64)
}

/// Decrypt peers' partials and finish, or refuse immediately for a roster below the minimum.
/// `local` is this finisher's private partial. `messages` has one packet from every peer.
pub fn finish(
    s: &Session,
    index: u32,
    keys: &SecretKeys,
    cs: &[Contribution],
    local: Option<&Partial>,
    messages: &[PartialMessage],
) -> Result<SignedOutcome> {
    let cs = verified_contributions(s, cs)?;
    s.identity(index, keys)?;
    let digest = contributions_digest(&cs);
    let outcome = if cs.len() < s.parameters.min_contributors as usize {
        Outcome::TooFewContributors
    } else {
        let local = local.ok_or_else(|| error(Some(index), "missing-partial"))?;
        guard(local.index == index, Some(index), "local-index")?;
        let mut total = check_partial(s, &cs, &digest, local)?;
        let mut seen = HashSet::from([index]);
        for message in messages {
            let j = message.index;
            s.party(j)?;
            guard(seen.insert(j), Some(j), "duplicate-partial")?;
            s.context(&message.session_id, &message.rule_digest, j)?;
            guard(message.recipient == index, Some(j), "partial-recipient")?;
            guard(
                message.contributions_digest == digest,
                Some(j),
                "contributions-digest",
            )?;
            let p: Partial = serde_json::from_slice(&open(
                s,
                b"partial",
                j,
                index,
                &digest,
                &message.packet,
                keys,
            )?)
            .map_err(|_| error(Some(j), "partial-encoding"))?;
            guard(p.index == j, Some(j), "partial-index")?;
            total.add(&check_partial(s, &cs, &digest, &p)?);
        }
        for j in 0..cs.len() as u32 {
            guard(seen.contains(&j), Some(j), "missing-partial")?;
        }
        let count = scalar_integer(total.c, cs.len() as u128 * u32::MAX as u128, index)? as u64;
        if count < s.parameters.min_group as u64 {
            let value = s.parameters.min_group as u64 - 1 - count;
            Outcome::BelowMinimumGroup {
                range_proof: prove(
                    s,
                    b"below-minimum",
                    index,
                    &[value],
                    &[-total.r],
                    32,
                    &mut OsRng,
                )?,
            }
        } else if let Some(t) = &s.parameters.threshold {
            threshold_outcome(s, t, &total, index)?
        } else {
            let sum = if s.has_sum() {
                Some(
                    scalar_integer(
                        total.s,
                        cs.len() as u128 * max_value(s.parameters.max_value_bits) as u128,
                        index,
                    )?
                    .to_string(),
                )
            } else {
                None
            };
            Outcome::Release {
                count,
                rho: sb(total.r),
                sum,
                sigma: s.has_sum().then(|| sb(total.t)),
            }
        }
    };
    let mut result = SignedOutcome {
        session_id: s.parameters.session_id.clone(),
        rule_digest: s.rule_digest.clone(),
        contributions_digest: digest,
        finisher: index,
        outcome,
        signature: Bytes::default(),
    };
    result.signature = sign(keys, &outcome_bytes(&result))?;
    Ok(result)
}

/// Independently verify all signed inputs and the output evidence. No partials are needed.
pub fn verify_outcome(s: &Session, cs: &[Contribution], output: &SignedOutcome) -> Result<()> {
    let cs = verified_contributions(s, cs)?;
    let i = output.finisher;
    s.party(i)?;
    s.context(&output.session_id, &output.rule_digest, i)?;
    guard(
        output.contributions_digest == contributions_digest(&cs),
        Some(i),
        "contributions-digest",
    )?;
    verify_signature(s.party(i)?, &outcome_bytes(output), &output.signature, i)?;
    let enough = cs.len() >= s.parameters.min_contributors as usize;
    if matches!(output.outcome, Outcome::TooFewContributors) {
        return guard(!enough, Some(i), "enough-contributors");
    }
    guard(enough, Some(i), "too-few-contributors")?;
    let (mut c, mut d) = (RistrettoPoint::identity(), RistrettoPoint::identity());
    for input in cs.iter() {
        let (a, b) = commitment_points(&input.commitment, s.has_sum(), input.index)?;
        c += a;
        d += b;
    }
    match &output.outcome {
        Outcome::Release {
            count,
            rho,
            sum,
            sigma,
        } => {
            guard(
                s.parameters.threshold.is_none(),
                Some(i),
                "threshold-required",
            )?;
            guard(
                *count >= s.parameters.min_group as u64,
                Some(i),
                "minimum-group",
            )?;
            guard(
                *count as u128 <= cs.len() as u128 * u32::MAX as u128,
                Some(i),
                "count-range",
            )?;
            guard(
                c == commit(Scalar::from(*count), scalar(rho, i)?),
                Some(i),
                "count-opening",
            )?;
            guard(
                sum.is_some() == s.has_sum() && sigma.is_some() == s.has_sum(),
                Some(i),
                "statistic-shape",
            )?;
            if let (Some(sum), Some(sigma)) = (sum, sigma) {
                let sum = decimal(sum, i)?;
                guard(
                    sum <= cs.len() as u128 * max_value(s.parameters.max_value_bits) as u128,
                    Some(i),
                    "sum-range",
                )?;
                let mut bytes = [0; 32];
                bytes[..16].copy_from_slice(&sum.to_le_bytes());
                guard(
                    d == commit(Scalar::from_bytes_mod_order(bytes), scalar(sigma, i)?),
                    Some(i),
                    "sum-opening",
                )?;
            }
            Ok(())
        }
        Outcome::BelowMinimumGroup { range_proof } => {
            guard(s.parameters.min_group > 0, Some(i), "minimum-disabled")?;
            let target = Scalar::from(s.parameters.min_group - 1) * pedersen().B - c;
            verify_proof(s, b"below-minimum", i, range_proof, &[target], 32)
        }
        Outcome::ThresholdVerdict {
            verdict,
            limb_commitments,
            range_proof,
        } => verify_threshold(s, i, *verdict, limb_commitments, range_proof, (c, d)),
        Outcome::TooFewContributors => unreachable!(),
    }
}

/// Simulate all contributors in one process through the real step functions.
/// The result includes private signed partials and is unsuitable for recipient delivery.
pub fn simulate_session(parameters: Parameters, parties: &[PartyInput]) -> Result<Simulation> {
    let session = Session::new(parameters)?;
    guard(
        parties.len() == session.parameters.roster.len(),
        None,
        "simulation-party-count",
    )?;
    let states = parties
        .iter()
        .enumerate()
        .map(|(i, p)| contribute(&session, i as u32, &p.keys, &p.input))
        .collect::<Result<Vec<_>>>()?;
    let contributions = states
        .iter()
        .map(|s| s.contribution.clone())
        .collect::<Vec<_>>();
    let partials = if parties.len() >= session.parameters.min_contributors as usize {
        parties
            .iter()
            .enumerate()
            .map(|(i, p)| {
                receive_and_partial(
                    &session,
                    i as u32,
                    &p.keys,
                    &contributions,
                    &states[i].local,
                )
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let messages = partials
        .iter()
        .flat_map(|p| p.messages.iter().filter(|m| m.recipient == 0).cloned())
        .collect::<Vec<_>>();
    let outcome = finish(
        &session,
        0,
        &parties[0].keys,
        &contributions,
        partials.first().map(|p| &p.local),
        &messages,
    )?;
    Ok(Simulation {
        session,
        contributions,
        partials,
        outcome,
    })
}

#[cfg(test)]
mod tests;
#[cfg(feature = "wasm")]
mod wasm;
