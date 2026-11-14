//! Verifier-side envelope policy words. Request normalization stays on the host.
//! Research grade and not externally audited; generated-layout wiring is separate.

use crate::{credential::field, interp::Word, statement::number, Error};
use oxrdf::{NamedNode, Term};
use sparq_merkle_suite::{poseidon as suite, Fr};
use sparq_zk::{field::field_to_be_bytes_32, poseidon2, sig};

const KEY_SET_LEAF: u64 = 0x5643515f454b4559;
const SNAPSHOT: u64 = 0x5643515f45534e50;
const STATUS_LEAF: u64 = 0x5643515f45534c46;
const HOLDER_REQUEST: u64 = 0x5643515f45484f4c;

/// Message signed by each required holder over the complete request digest.
pub fn holder_message(digest: &[u8; 32]) -> Fr {
    let high = u128::from_be_bytes(digest[..16].try_into().unwrap());
    let low = u128::from_be_bytes(digest[16..].try_into().unwrap());
    poseidon2::hash(&[Fr::from(HOLDER_REQUEST), high.into(), low.into()])
}

/// Tagged status bit, before ordered status-tree hashing.
pub fn status_leaf(bit: bool) -> Fr {
    poseidon2::hash(&[
        Fr::from(STATUS_LEAF),
        Fr::from(u64::from(bit)),
        Fr::from(0u64),
    ])
}

/// Match the envelope snapshot encoding after snapshot authentication.
/// List and purpose are suite term encodings; depth is the status-bit tree depth.
/// This encoding alone does not authenticate or admit a status list.
pub fn snapshot_digest(
    list: Word,
    purpose: Word,
    bit_len: u32,
    depth: u32,
    root: Word,
) -> Result<Word, Error> {
    Ok(suite::field_bytes(poseidon2::hash(&[
        Fr::from(SNAPSHOT),
        field(list)?,
        field(purpose)?,
        Fr::from(bit_len),
        Fr::from(depth),
        field(root)?,
    ])))
}

/// An issuer, its assertion verification method and the authorized suite key.
#[derive(Clone, Debug)]
pub struct TrustMember {
    pub issuer: NamedNode,
    pub verification_method: NamedNode,
    pub key: sig::PublicKey,
}

/// Presence enables holder binding. An absent issuer list selects all issuers.
/// A present issuer list must be nonempty; omit holder binding to require no keys.
#[derive(Clone, Debug, Default)]
pub struct HolderBinding {
    pub issuers: Option<Vec<NamedNode>>,
}

/// Snapshot digests already authenticated and curated by the verifier.
#[derive(Clone, Debug)]
pub struct AcceptedSnapshots {
    pub digests: Vec<Word>,
    pub allow_missing: bool,
}

/// Typed policy properties extracted from the immutable stored request.
/// Instants are exact nonnegative Unix epoch milliseconds, without rounding.
/// The admitted range matches the suite's signed millisecond timestamps.
#[derive(Clone, Debug, Default)]
pub struct PolicyInput {
    pub trust_members: Vec<TrustMember>,
    pub holder_binding: Option<HolderBinding>,
    pub accepted_snapshots: Option<AcceptedSnapshots>,
    pub validity_time: Option<u64>,
    pub earliest_created: Option<u64>,
}

/// A minimal, zero-padded set tree and its sorted, unique canonical members.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalSet {
    pub root: Word,
    pub depth: u32,
    pub members: Vec<Word>,
}

/// Sort canonical unsigned field bytes, collapse duplicates and build the least
/// binary tree with zero padding. Empty sets and zero members are refused.
pub fn canonical_set_root(members: &[Word]) -> Result<CanonicalSet, Error> {
    Ok(build_canonical_set(members, None)?.0)
}

/// Return the canonical set, selected leaf index and siblings from leaf to root.
/// The same builder fixes validation, ordering, deduplication, padding and depth
/// for roots and membership paths. A member absent from the set is refused.
pub fn canonical_set_membership(
    members: &[Word],
    member: Word,
) -> Result<(CanonicalSet, u32, Vec<Word>), Error> {
    build_canonical_set(members, Some(member))
}

fn build_canonical_set(
    members: &[Word],
    member: Option<Word>,
) -> Result<(CanonicalSet, u32, Vec<Word>), Error> {
    if members.is_empty() {
        return Err(Error::Invalid("empty policy set".into()));
    }
    let mut members = members.to_vec();
    for &word in &members {
        field(word)?;
        if word == [0; 32] {
            return Err(Error::Invalid("zero policy set member".into()));
        }
    }
    members.sort_unstable();
    members.dedup();
    let width = members
        .len()
        .checked_next_power_of_two()
        .ok_or_else(|| Error::Capacity("policy set tree".into()))?;
    let selected = if let Some(member) = member {
        field(member)?;
        members
            .binary_search(&member)
            .map_err(|_| Error::Invalid("policy set member absent".into()))?
    } else {
        0
    };
    let index =
        u32::try_from(selected).map_err(|_| Error::Capacity("policy set member index".into()))?;
    let mut position = selected;
    let mut siblings = Vec::new();
    let depth = width.trailing_zeros();
    let mut row = members
        .iter()
        .copied()
        .map(field)
        .collect::<Result<Vec<_>, _>>()?;
    row.resize(width, Fr::from(0u64));
    while row.len() > 1 {
        if member.is_some() {
            siblings.push(field_to_be_bytes_32(&row[position ^ 1]));
            position >>= 1;
        }
        row = row
            .chunks_exact(2)
            .map(|p| poseidon2::hash(&[p[0], p[1]]))
            .collect();
    }
    let root = field_to_be_bytes_32(&row[0]);
    if root == [0; 32] {
        return Err(Error::Invalid("zero policy root".into()));
    }
    Ok((
        CanonicalSet {
            root,
            depth,
            members,
        },
        index,
        siblings,
    ))
}

/// Use the suite IRI encoding without spelling changes.
pub fn issuer_encoding(issuer: &NamedNode) -> Result<Word, Error> {
    let cell = suite::term_cell(&Term::NamedNode(issuer.clone()), 0)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(suite::field_bytes(cell.enc))
}

/// Match the envelope's identity/method/suite/assertion/key leaf exactly.
pub fn trust_member_digest(member: &TrustMember) -> Result<Word, Error> {
    if !member.key.is_prime_order() {
        return Err(Error::Invalid("issuer key".into()));
    }
    let assertion = NamedNode::new_unchecked("https://w3id.org/security#assertionMethod");
    let key = poseidon2::hash(&[suite::suite_id(), member.key.0.x, member.key.0.y]);
    Ok(suite::field_bytes(poseidon2::hash(&[
        Fr::from(KEY_SET_LEAF),
        field(issuer_encoding(&member.issuer)?)?,
        field(issuer_encoding(&member.verification_method)?)?,
        key,
        field(issuer_encoding(&assertion)?)?,
    ])))
}

/// The agreed fourteen public groups, in declaration order. The issuer list
/// flattens to four field words, making seventeen scalar words in total.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicPolicy {
    pub issuer_root: Word,
    pub issuer_depth: u32,
    pub holder_required: bool,
    pub holder_issuers_present: bool,
    pub holder_issuer_count: u32,
    pub holder_issuers: [Word; 4],
    pub status_required: bool,
    pub allow_missing_status: bool,
    pub snapshot_root: Word,
    pub snapshot_depth: u32,
    pub validity_required: bool,
    pub now: u64,
    pub freshness_required: bool,
    pub earliest_created: u64,
}
impl PublicPolicy {
    /// Canonical big-endian field words for the one authenticated public layout.
    pub fn words(&self) -> [Word; 17] {
        [
            self.issuer_root,
            number(u64::from(self.issuer_depth)),
            number(u64::from(self.holder_required)),
            number(u64::from(self.holder_issuers_present)),
            number(u64::from(self.holder_issuer_count)),
            self.holder_issuers[0],
            self.holder_issuers[1],
            self.holder_issuers[2],
            self.holder_issuers[3],
            number(u64::from(self.status_required)),
            number(u64::from(self.allow_missing_status)),
            self.snapshot_root,
            number(u64::from(self.snapshot_depth)),
            number(u64::from(self.validity_required)),
            number(self.now),
            number(u64::from(self.freshness_required)),
            number(self.earliest_created),
        ]
    }
}

/// Derive roots, depths, flags and canonical zero values from stored policy input.
/// This function does not accept roots or flags chosen by a presentation.
pub fn derive(input: &PolicyInput) -> Result<PublicPolicy, Error> {
    if [input.validity_time, input.earliest_created]
        .into_iter()
        .flatten()
        .any(|instant| instant > i64::MAX as u64)
    {
        return Err(Error::Invalid("envelope instant range".into()));
    }
    let trust = input
        .trust_members
        .iter()
        .map(trust_member_digest)
        .collect::<Result<Vec<_>, _>>()?;
    let issuer = canonical_set_root(&trust)?;
    let mut policy = PublicPolicy {
        issuer_root: issuer.root,
        issuer_depth: issuer.depth,
        holder_required: input.holder_binding.is_some(),
        holder_issuers_present: false,
        holder_issuer_count: 0,
        holder_issuers: [[0; 32]; 4],
        status_required: input.accepted_snapshots.is_some(),
        allow_missing_status: false,
        snapshot_root: [0; 32],
        snapshot_depth: 0,
        validity_required: input.validity_time.is_some(),
        now: input.validity_time.unwrap_or(0),
        freshness_required: input.earliest_created.is_some(),
        earliest_created: input.earliest_created.unwrap_or(0),
    };
    if let Some(HolderBinding {
        issuers: Some(issuers),
    }) = &input.holder_binding
    {
        if issuers.is_empty() {
            return Err(Error::Invalid("empty holder issuer list".into()));
        }
        if issuers.len() > 4 {
            return Err(Error::Capacity("holder issuer list".into()));
        }
        let mut encodings = issuers
            .iter()
            .map(issuer_encoding)
            .collect::<Result<Vec<_>, _>>()?;
        if encodings.contains(&[0; 32]) {
            return Err(Error::Invalid("zero holder issuer encoding".into()));
        }
        encodings.sort_unstable();
        encodings.dedup();
        policy.holder_issuers_present = true;
        policy.holder_issuer_count = encodings.len() as u32;
        policy.holder_issuers[..encodings.len()].copy_from_slice(&encodings);
    }
    if let Some(snapshots) = &input.accepted_snapshots {
        let snapshot = canonical_set_root(&snapshots.digests)?;
        policy.snapshot_root = snapshot.root;
        policy.snapshot_depth = snapshot.depth;
        policy.allow_missing_status = snapshots.allow_missing;
    }
    Ok(policy)
}

/// Return the complete flattened policy sequence with verifier-computed roots.
pub fn public_words(input: &PolicyInput) -> Result<[Word; 17], Error> {
    Ok(derive(input)?.words())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_zk::field::field_from_hex_str_strict;

    #[test]
    fn envelope_protocol_constants_equal_their_rust_sources() {
        let source = include_str!("../../../zk/noir-query/core/src/envelope.nr");
        let mut actual = std::collections::BTreeMap::new();
        for line in source.lines() {
            let code = line.split("//").next().unwrap().trim();
            if let Some(declaration) = code.strip_prefix("global ") {
                let (name, rest) = declaration.split_once(':').unwrap();
                let (kind, rest) = rest.split_once('=').unwrap();
                assert_eq!(kind.trim(), "Field");
                let value = rest.trim().strip_suffix(';').unwrap().trim();
                let value = u64::from_str_radix(value.strip_prefix("0x").unwrap(), 16).unwrap();
                assert!(actual.insert(name.trim(), value).is_none());
            }
        }
        let expected = std::collections::BTreeMap::from([
            ("KEY_SET_LEAF", KEY_SET_LEAF),
            ("SNAPSHOT", SNAPSHOT),
            ("STATUS_LEAF", STATUS_LEAF),
            ("HOLDER_REQUEST", HOLDER_REQUEST),
        ]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn envelope_encodings_bind_each_input() {
        let digest = std::array::from_fn(|i| i as u8);
        let message = holder_message(&digest);
        for i in 0..digest.len() {
            let mut changed = digest;
            changed[i] ^= 1;
            assert_ne!(message, holder_message(&changed));
        }
        assert_ne!(status_leaf(false), status_leaf(true));
        let snapshot = snapshot_digest(number(11), number(13), 4, 2, number(17)).unwrap();
        for changed in [
            snapshot_digest(number(19), number(13), 4, 2, number(17)),
            snapshot_digest(number(11), number(23), 4, 2, number(17)),
            snapshot_digest(number(11), number(13), 8, 2, number(17)),
            snapshot_digest(number(11), number(13), 4, 3, number(17)),
            snapshot_digest(number(11), number(13), 4, 2, number(29)),
        ] {
            assert_ne!(snapshot, changed.unwrap());
        }
        for (list, purpose, root) in [
            ([255; 32], number(13), number(17)),
            (number(11), [255; 32], number(17)),
            (number(11), number(13), [255; 32]),
        ] {
            assert!(snapshot_digest(list, purpose, 4, 2, root).is_err());
        }
    }

    fn root_from_path(leaf: Word, index: u32, siblings: &[Word], depth: u32) -> Word {
        assert_eq!(siblings.len(), depth as usize);
        assert_eq!((index as u64) >> depth, 0);
        let mut node = field(leaf).unwrap();
        for (level, sibling) in siblings.iter().enumerate() {
            let sibling = field(*sibling).unwrap();
            let pair = if (index >> level) & 1 == 0 {
                [node, sibling]
            } else {
                [sibling, node]
            };
            node = poseidon2::hash(&pair);
        }
        field_to_be_bytes_32(&node)
    }

    #[test]
    fn canonical_membership_matches_roots_for_every_small_set_member() {
        for size in 1..=9 {
            let members = (1..=size).map(|n| number(3 * n + 1)).collect::<Vec<_>>();
            let expected = canonical_set_root(&members).unwrap();
            let mut reordered = members.clone();
            reordered.reverse();
            reordered.extend_from_slice(&members);
            for (position, &member) in expected.members.iter().enumerate() {
                let (set, index, siblings) = canonical_set_membership(&members, member).unwrap();
                assert_eq!(set, expected);
                assert_eq!(index as usize, position);
                assert_eq!(siblings.len(), set.depth as usize);
                assert_eq!(
                    root_from_path(member, index, &siblings, set.depth),
                    set.root
                );
                assert_eq!(
                    canonical_set_membership(&reordered, member).unwrap(),
                    (set.clone(), index, siblings.clone())
                );
                if !siblings.is_empty() {
                    let mut changed = siblings;
                    changed[0] = number(999);
                    assert_ne!(root_from_path(member, index, &changed, set.depth), set.root);
                }
            }
        }
    }

    #[test]
    fn canonical_membership_uses_the_existing_noir_golden_roots() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/envelope_set_roots.json"))
                .unwrap();
        for vector in vectors["sets"].as_array().unwrap() {
            let members = vector["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| parse(v.as_str().unwrap()))
                .collect::<Vec<_>>();
            for member in &members {
                let (set, index, siblings) = canonical_set_membership(&members, *member).unwrap();
                assert_eq!(set.root, parse(vector["root"].as_str().unwrap()));
                assert_eq!(set.depth, vector["depth"].as_u64().unwrap() as u32);
                assert_eq!(
                    root_from_path(*member, index, &siblings, set.depth),
                    set.root
                );
            }
        }
    }

    #[test]
    fn canonical_membership_refuses_absent_members_and_invalid_sets() {
        assert!(
            matches!(canonical_set_membership(&[number(1), number(2)], number(3)), Err(Error::Invalid(reason)) if reason == "policy set member absent")
        );
        for members in [vec![], vec![[0; 32]], vec![[255; 32]]] {
            assert!(canonical_set_root(&members).is_err());
            assert!(canonical_set_membership(&members, number(1)).is_err());
        }
        assert!(canonical_set_membership(&[number(1)], [255; 32]).is_err());
        let (set, index, siblings) =
            canonical_set_membership(&[number(7), number(7)], number(7)).unwrap();
        assert_eq!(
            (set.root, set.depth, index, siblings),
            (number(7), 0, 0, vec![])
        );
    }

    fn iri(text: &str) -> NamedNode {
        NamedNode::new(text).unwrap()
    }
    fn member() -> TrustMember {
        TrustMember {
            issuer: iri("urn:test:issuer"),
            verification_method: iri("urn:test:key"),
            key: sig::SecretKey(1u64.into()).public_key(),
        }
    }
    fn input() -> PolicyInput {
        PolicyInput {
            trust_members: vec![member()],
            ..PolicyInput::default()
        }
    }
    fn parse(text: &str) -> Word {
        field_to_be_bytes_32(&field_from_hex_str_strict(text).unwrap())
    }
    fn snapshots(digests: &[u64], allow_missing: bool) -> AcceptedSnapshots {
        AcceptedSnapshots {
            digests: digests.iter().copied().map(number).collect(),
            allow_missing,
        }
    }

    #[test]
    fn issued_credential_trust_member_matches_noir_vector() {
        let key = suite::keypair_from_seed(&[1; 32]);
        let document = "<urn:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer:1> .\n";
        let configuration = "<urn:proof> <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n<urn:proof> <https://w3id.org/security#verificationMethod> <urn:method:1> .\n<urn:proof> <http://purl.org/dc/terms/created> \"1970-01-01T00:00:00.101Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n";
        let credential =
            suite::issue_deterministic(document, configuration, &key, &[31; 32]).unwrap();
        let opened = suite::verify_and_open(&credential, &key.public_key()).unwrap();
        assert_eq!(
            sparq_merkle_suite::metadata::timestamp(opened.created.as_ref()),
            Some(101)
        );
        let issuer = oxttl::NQuadsParser::new()
            .for_slice(credential.document.as_bytes())
            .map(Result::unwrap)
            .find(|q| q.predicate.as_str() == "https://www.w3.org/2018/credentials#issuer")
            .unwrap()
            .object;
        let Term::NamedNode(issuer) = issuer else {
            panic!("issuer IRI")
        };
        let Some(Term::NamedNode(method)) = opened.verification_method else {
            panic!("method IRI")
        };
        let member = TrustMember {
            issuer,
            verification_method: method,
            key: key.public_key(),
        };
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/envelope_set_roots.json"))
                .unwrap();
        assert_eq!(
            trust_member_digest(&member).unwrap(),
            parse(vectors["issued_trust_member"].as_str().unwrap())
        );
        let mut changed = member.clone();
        changed.verification_method = iri("urn:method:other");
        assert_ne!(
            trust_member_digest(&member).unwrap(),
            trust_member_digest(&changed).unwrap()
        );
    }

    #[test]
    fn millisecond_instants_are_not_rounded() {
        let mut input = input();
        input.validity_time = Some(101);
        input.earliest_created = Some(99);
        let words = public_words(&input).unwrap();
        assert_eq!(words[14], number(101));
        assert_eq!(words[16], number(99));
        assert_ne!(words[14], number(0));
        for instant in [i64::MAX as u64 + 1, u64::MAX] {
            input.validity_time = Some(instant);
            assert!(
                matches!(derive(&input), Err(Error::Invalid(reason)) if reason == "envelope instant range")
            );
        }
        input.validity_time = Some(101);
        input.earliest_created = Some(i64::MAX as u64 + 1);
        assert!(
            matches!(derive(&input), Err(Error::Invalid(reason)) if reason == "envelope instant range")
        );
    }

    #[test]
    fn fixed_noir_native_vectors_match_both_orders_duplicates_and_capacity() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/envelope_set_roots.json"))
                .unwrap();
        for vector in vectors["sets"].as_array().unwrap() {
            let members = vector["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| parse(x.as_str().unwrap()))
                .collect::<Vec<_>>();
            let actual = canonical_set_root(&members).unwrap();
            assert_eq!(
                actual.root,
                parse(vector["root"].as_str().unwrap()),
                "{}",
                vector["name"]
            );
            assert_eq!(actual.depth, vector["depth"].as_u64().unwrap() as u32);
            assert_eq!(
                actual.members.len(),
                vector["unique"].as_u64().unwrap() as usize
            );
            assert!(vector["noir_capacity"].as_u64().unwrap() >= members.len() as u64);
        }
        assert_eq!(
            trust_member_digest(&member()).unwrap(),
            parse(vectors["trust_member"].as_str().unwrap())
        );
    }

    #[test]
    fn full_policy_matches_fixed_native_roots_and_public_words() {
        let mut request = input();
        request.holder_binding = Some(HolderBinding::default());
        request.accepted_snapshots = Some(snapshots(&[2, 1, 2], false));
        request.validity_time = Some(150);
        request.earliest_created = Some(100);
        let mut expected = [0; 17].map(number);
        expected[0] = parse("0x0634ae948a9551ef686ae6afd30505a0ab76509e71b977393210dc7e764dd62c");
        expected[2] = number(1);
        expected[9] = number(1);
        expected[11] = parse("0x038682aa1cb5ae4e0a3f13da432a95c77c5c111f6f030faf9cad641ce1ed7383");
        expected[12] = number(1);
        expected[13] = number(1);
        expected[14] = number(150);
        expected[15] = number(1);
        expected[16] = number(100);
        assert_eq!(public_words(&request).unwrap(), expected);
    }

    #[test]
    fn public_word_order_matches_the_agreed_layout() {
        let policy = PublicPolicy {
            issuer_root: number(11),
            issuer_depth: 2,
            holder_required: true,
            holder_issuers_present: true,
            holder_issuer_count: 2,
            holder_issuers: [number(21), number(22), [0; 32], [0; 32]],
            status_required: true,
            allow_missing_status: true,
            snapshot_root: number(31),
            snapshot_depth: 3,
            validity_required: true,
            now: 41,
            freshness_required: true,
            earliest_created: 51,
        };
        let expected = [11, 2, 1, 1, 2, 21, 22, 0, 0, 1, 1, 31, 3, 1, 41, 1, 51].map(number);
        assert_eq!(policy.words(), expected);
    }

    #[test]
    fn each_optional_property_occupies_its_documented_word_positions() {
        let base = input();
        let mut baseline = [[0; 32]; 17];
        baseline[0] = parse("0x0634ae948a9551ef686ae6afd30505a0ab76509e71b977393210dc7e764dd62c");
        let mut holder = base.clone();
        holder.holder_binding = Some(HolderBinding::default());
        let mut expected = baseline;
        expected[2] = number(1);
        assert_eq!(public_words(&holder).unwrap(), expected);
        holder.holder_binding.as_mut().unwrap().issuers = Some(vec![iri("urn:test:issuer")]);
        expected[3] = number(1);
        expected[4] = number(1);
        expected[5] = issuer_encoding(&iri("urn:test:issuer")).unwrap();
        assert_eq!(public_words(&holder).unwrap(), expected);
        let mut status = base.clone();
        status.accepted_snapshots = Some(snapshots(&[1], true));
        expected = baseline;
        expected[9] = number(1);
        expected[10] = number(1);
        expected[11] = number(1);
        assert_eq!(public_words(&status).unwrap(), expected);
        let mut validity = base.clone();
        validity.validity_time = Some(41);
        expected = baseline;
        expected[13] = number(1);
        expected[14] = number(41);
        assert_eq!(public_words(&validity).unwrap(), expected);
        let mut freshness = base;
        freshness.earliest_created = Some(51);
        expected = baseline;
        expected[15] = number(1);
        expected[16] = number(51);
        assert_eq!(public_words(&freshness).unwrap(), expected);
    }

    #[test]
    fn disabled_properties_emit_zero_words() {
        let policy = derive(&input()).unwrap();
        assert_eq!(&policy.words()[2..], &[[0; 32]; 15]);
        assert_eq!(policy.issuer_depth, 0);
        assert_eq!(policy.issuer_root, trust_member_digest(&member()).unwrap());
    }

    #[test]
    fn requests_differing_in_one_policy_property_have_different_public_words() {
        let base = input();
        let mut different = base.clone();
        different.trust_members[0].issuer = iri("urn:test:other-issuer");
        assert_ne!(
            public_words(&base).unwrap(),
            public_words(&different).unwrap()
        );
        let mut all = base.clone();
        all.holder_binding = Some(HolderBinding::default());
        assert_ne!(public_words(&base).unwrap(), public_words(&all).unwrap());
        let mut listed = all.clone();
        listed.holder_binding.as_mut().unwrap().issuers = Some(vec![iri("urn:test:issuer")]);
        assert_ne!(public_words(&all).unwrap(), public_words(&listed).unwrap());
        let mut other = listed.clone();
        other.holder_binding.as_mut().unwrap().issuers = Some(vec![iri("urn:test:other-issuer")]);
        assert_ne!(
            public_words(&listed).unwrap(),
            public_words(&other).unwrap()
        );
        let mut status = base.clone();
        status.accepted_snapshots = Some(snapshots(&[1], false));
        assert_ne!(public_words(&base).unwrap(), public_words(&status).unwrap());
        other = status.clone();
        other.accepted_snapshots.as_mut().unwrap().allow_missing = true;
        assert_ne!(
            public_words(&status).unwrap(),
            public_words(&other).unwrap()
        );
        other = status.clone();
        other.accepted_snapshots = Some(snapshots(&[1, 2], false));
        assert_ne!(
            public_words(&status).unwrap(),
            public_words(&other).unwrap()
        );
        for (a, b) in [(None, Some(0)), (Some(100), Some(101))] {
            let mut first = base.clone();
            first.validity_time = a;
            other = first.clone();
            other.validity_time = b;
            assert_ne!(public_words(&first).unwrap(), public_words(&other).unwrap());
            first = base.clone();
            first.earliest_created = a;
            other = first.clone();
            other.earliest_created = b;
            assert_ne!(public_words(&first).unwrap(), public_words(&other).unwrap());
        }
    }

    #[test]
    fn set_enumerations_and_duplicate_members_do_not_change_public_words() {
        let mut a = input();
        a.trust_members.push(TrustMember {
            issuer: iri("urn:test:second"),
            ..member()
        });
        a.accepted_snapshots = Some(snapshots(&[1, 2], false));
        let mut b = a.clone();
        b.trust_members.reverse();
        b.trust_members.push(member());
        b.accepted_snapshots = Some(snapshots(&[2, 1, 2], false));
        assert_eq!(public_words(&a).unwrap(), public_words(&b).unwrap());
        assert_eq!(derive(&a).unwrap().issuer_depth, 1);
        assert_eq!(derive(&a).unwrap().snapshot_depth, 1);
    }

    #[test]
    fn holder_scope_is_sorted_unique_zero_padded_and_preserves_exact_iris() {
        let mut a = input();
        a.holder_binding = Some(HolderBinding {
            issuers: Some(vec![iri("did:web:Issuer.example"), iri("urn:test:issuer")]),
        });
        let mut b = a.clone();
        b.holder_binding.as_mut().unwrap().issuers = Some(vec![
            iri("urn:test:issuer"),
            iri("did:web:Issuer.example"),
            iri("urn:test:issuer"),
        ]);
        let policy = derive(&a).unwrap();
        assert_eq!(public_words(&a).unwrap(), public_words(&b).unwrap());
        assert_eq!(policy.holder_issuer_count, 2);
        assert!(policy.holder_issuers[0] < policy.holder_issuers[1]);
        assert_eq!(&policy.holder_issuers[2..], &[[0; 32]; 2]);
        assert_ne!(
            issuer_encoding(&iri("did:web:Issuer.example")).unwrap(),
            issuer_encoding(&iri("did:web:issuer.example")).unwrap()
        );
    }

    #[test]
    fn empty_holder_issuer_list_is_refused_by_name() {
        let mut request = input();
        request.holder_binding = Some(HolderBinding {
            issuers: Some(vec![]),
        });
        assert!(matches!(public_words(&request), Err(Error::Invalid(reason))
            if reason == "empty holder issuer list"));
    }

    #[test]
    fn holder_scope_above_capacity_is_refused_before_deduplication() {
        let mut a = input();
        a.holder_binding = Some(HolderBinding {
            issuers: Some(vec![iri("urn:test:issuer"); 5]),
        });
        assert!(matches!(derive(&a), Err(Error::Capacity(_))));
    }

    #[test]
    fn empty_noncanonical_and_zero_policy_members_are_refused() {
        assert!(canonical_set_root(&[]).is_err());
        assert!(canonical_set_root(&[[0; 32]]).is_err());
        assert!(canonical_set_root(&[[255; 32]]).is_err());
        let mut a = input();
        a.accepted_snapshots = Some(snapshots(&[], false));
        assert!(derive(&a).is_err());
        a.accepted_snapshots = Some(snapshots(&[0], false));
        assert!(derive(&a).is_err());
        a.trust_members.clear();
        assert!(derive(&a).is_err());
    }

    #[test]
    fn trust_member_binds_every_identity_and_key_component() {
        let original = member();
        let leaf = trust_member_digest(&original).unwrap();
        let mut other = original.clone();
        other.issuer = iri("urn:test:other-issuer");
        assert_ne!(leaf, trust_member_digest(&other).unwrap());
        other = original.clone();
        other.verification_method = iri("urn:test:other-key");
        assert_ne!(leaf, trust_member_digest(&other).unwrap());
        other = original.clone();
        other.key = sig::SecretKey(2u64.into()).public_key();
        assert_ne!(leaf, trust_member_digest(&other).unwrap());
        other = original;
        other.key = sig::SecretKey(0u64.into()).public_key();
        assert!(trust_member_digest(&other).is_err());
    }
}
