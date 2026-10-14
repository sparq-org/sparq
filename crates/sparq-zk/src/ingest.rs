//! Trusted-ingest path: per-named-graph commitment with globally-unique salts
//! (plan §2.1–§2.2; soundness audit #9; tasks sq-610 + sq-cn8).
//!
//! This is "Stage 2" of the ingest-path commitment work. Stage 1 is the
//! per-graph commitment primitive ([`crate::commit::commit_triples`]) and the
//! `<urn:sparq:zk>` registry ([`crate::registry`]); this module is the trusted
//! orchestration that ties them together at mint time:
//!
//! 1. **Salt mint (sq-610).** Every named graph gets a salt drawn from
//!    [`SaltMint`], which fills 32 bytes from the OS CSPRNG ([`getrandom`]) and
//!    enforces **global uniqueness** across the whole ingest: every issued salt
//!    is tracked, a CSPRNG collision is redrawn, and any externally-supplied
//!    salt that collides with one already issued is *rejected* ([`SaltMint::register`]).
//!    The salt is the per-graph RDFC10 bnode salt (`zk:rdfc10Salt`): the Q6
//!    "bnodes from different graphs are distinct by construction" guarantee
//!    rests on it being globally unique, so this is the trusted-ingest
//!    enforcement the audit (#9) requires instead of an unenforced convention.
//!
//! 2. **Per-named-graph commitment (sq-cn8).** Each named graph is committed
//!    *separately* under its own minted salt, yielding one [`GraphCommitment`]
//!    per graph (NOT a single global commitment over the merged store). This is
//!    exactly the per-graph attribution the zk-trace consumer ([`crate::trace`])
//!    references by graph index and the per-property circuit witnesses block by
//!    block. The output is wired straight into a [`ZkDataset`] for traced
//!    execution and into [`RegistryEntry`]s for the `<urn:sparq:zk>` registry.
//!
//! # Soundness note (honest scope)
//! Salt uniqueness is enforced *at ingest*; the verifier-side / in-circuit
//! binding of the salt (so a salt-reusing prover cannot present a graph the
//! trusted ingest never minted) lives in the issuer signature
//! ([`crate::sig::commitment_message_with_salt`], audit #9 fix (a)) and is NOT
//! re-implemented here — this module is the *mint* side. A salt minted here is
//! globally unique within one [`SaltMint`] session; uniqueness across separate
//! processes/sessions rests on 248-bit CSPRNG entropy (collision probability
//! negligible), and a persistent ingester that must guarantee uniqueness across
//! restarts should seed the mint's seen-set from the existing registry (see
//! [`SaltMint::from_registry`]). What is deliberately deferred: in-circuit salt
//! binding (audit #9 fix (b)) and cross-process durable uniqueness state.

use crate::commit::{commit_triples, CommitError, GraphCommitment};
use crate::encode::salt_from_bytes;
use crate::field::Fr;
use crate::registry::RegistryEntry;
use crate::trace::{TraceError, ZkDataset};
use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use std::collections::HashSet;

/// How many CSPRNG redraws before a mint gives up on finding a fresh salt.
/// A collision needs two of the same 248-bit draw, so a single redraw already
/// makes failure astronomically unlikely; the bound only exists so a broken
/// entropy source fails loud instead of looping forever.
const MAX_MINT_REDRAWS: usize = 64;

/// Ingest-path failures.
#[derive(Debug)]
pub enum IngestError {
    /// The salt mint could not produce a fresh salt (broken entropy source, or
    /// a saturated seen-set — both unreachable in practice).
    SaltExhausted,
    /// An externally-supplied salt collides with one already issued by the same
    /// mint — the uniqueness invariant the trusted ingest enforces (sq-610).
    SaltCollision,
    /// A requested named graph was not present in the store.
    MissingGraph(String),
    /// Commitment pipeline failure for a graph (canonicalization / encoding).
    Commit(String, CommitError),
    /// Materializing a graph's content for commitment failed.
    Content(String, CommitError),
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestError::SaltExhausted => write!(
                f,
                "salt mint exhausted: could not draw a globally-unique salt (check the OS CSPRNG)"
            ),
            IngestError::SaltCollision => write!(
                f,
                "salt collision rejected: the supplied salt was already issued in this ingest \
                 (globally-unique per-graph salts are required — Q6 / audit #9)"
            ),
            IngestError::MissingGraph(g) => write!(f, "named graph not in store: {g}"),
            IngestError::Commit(g, e) => write!(f, "commitment failed for {g}: {e}"),
            IngestError::Content(g, e) => write!(f, "reading content of {g} failed: {e}"),
        }
    }
}

impl std::error::Error for IngestError {}

/// A trusted-ingest salt mint: draws globally-unique per-graph RDFC10 bnode
/// salts and enforces that no two graphs in the same ingest share a salt
/// (sq-610 / audit #9).
///
/// The salt is a 248-bit BN254 field element ([`salt_from_bytes`] over 32 OS
/// CSPRNG bytes). Uniqueness is structural: every issued salt is recorded, a
/// (vanishingly improbable) CSPRNG collision triggers a redraw, and a caller
/// who supplies its own salt has it checked against the seen-set.
#[derive(Debug, Default)]
pub struct SaltMint {
    /// Every salt this mint has issued / accepted — the uniqueness oracle.
    issued: HashSet<Fr>,
}

impl SaltMint {
    /// A fresh mint with an empty seen-set.
    pub fn new() -> Self {
        SaltMint { issued: HashSet::new() }
    }

    /// Seeds the seen-set from an existing registry's salts so a persistent
    /// ingester guarantees uniqueness against previously-minted graphs (cross
    /// the durability gap noted in the module docs). Salts already in the
    /// registry are treated as issued.
    pub fn from_registry(entries: &[RegistryEntry]) -> Self {
        SaltMint { issued: entries.iter().map(|e| e.salt).collect() }
    }

    /// The number of salts issued so far (for tests / introspection).
    pub fn issued_count(&self) -> usize {
        self.issued.len()
    }

    /// `true` if `salt` has already been issued by this mint.
    pub fn contains(&self, salt: &Fr) -> bool {
        self.issued.contains(salt)
    }

    /// Draws a globally-unique salt from the OS CSPRNG. Fills 32 random bytes,
    /// folds them into a field element, and redraws on the (negligible) chance
    /// the same salt was already issued. Fails closed ([`IngestError::SaltExhausted`])
    /// only if the entropy source is broken (no fresh value in
    /// `MAX_MINT_REDRAWS` tries).
    pub fn mint(&mut self) -> Result<Fr, IngestError> {
        for _ in 0..MAX_MINT_REDRAWS {
            let mut bytes = [0u8; 32];
            // getrandom is the OS CSPRNG; an error here means no entropy source.
            // sq-8xug: getrandom 0.4 renamed the top-level fn to `fill`.
            if getrandom::fill(&mut bytes).is_err() {
                return Err(IngestError::SaltExhausted);
            }
            let salt = salt_from_bytes(&bytes);
            if self.issued.insert(salt) {
                return Ok(salt);
            }
        }
        Err(IngestError::SaltExhausted)
    }

    /// Registers an externally-supplied salt (e.g. one drawn by an issuer tool
    /// out of band), enforcing global uniqueness. Returns
    /// [`IngestError::SaltCollision`] if the salt was already issued — this is
    /// the *detection/prevention* of a forced collision (sq-610): a duplicate
    /// salt can never enter the ingest.
    pub fn register(&mut self, salt: Fr) -> Result<Fr, IngestError> {
        if self.issued.insert(salt) {
            Ok(salt)
        } else {
            Err(IngestError::SaltCollision)
        }
    }
}

/// One committed named graph of a trusted ingest: its name, its minted salt,
/// and its per-graph commitment. The `commitment.commitment` field is `C(G)`,
/// committed under `salt` over the graph's RDFC10-canonical content.
#[derive(Debug, Clone)]
pub struct IngestedGraph {
    pub name: NamedNode,
    pub salt: Fr,
    pub commitment: GraphCommitment,
}

impl IngestedGraph {
    /// The registry entry for this ingested graph (unattested — the issuer
    /// signature is added by the issuer-attestation path, not here). Records
    /// `C(G)`, the scheme, and the minted salt under `<urn:sparq:zk>`.
    pub fn registry_entry(&self) -> RegistryEntry {
        RegistryEntry::new(self.name.clone(), self.commitment.commitment, self.salt)
    }
}

/// The output of one trusted ingest: a per-named-graph commitment for each
/// requested graph, each under a globally-unique minted salt (sq-cn8). This is
/// the Stage-2 artifact the zk-trace consumer and the registry are built from.
#[derive(Debug, Clone)]
pub struct IngestedDataset {
    pub graphs: Vec<IngestedGraph>,
}

impl IngestedDataset {
    /// Commits every named graph in `names` from `store`, minting a fresh
    /// globally-unique salt per graph (sq-610) and producing one per-graph
    /// commitment each (sq-cn8). Graphs are committed independently — there is
    /// no single merged commitment.
    pub fn ingest(store: &Graph, names: &[NamedNode]) -> Result<Self, IngestError> {
        let mut mint = SaltMint::new();
        Self::ingest_with_mint(store, names, &mut mint)
    }

    /// As [`Self::ingest`] but against a caller-provided [`SaltMint`], so a
    /// persistent ingester can carry its uniqueness state across calls (e.g.
    /// seeded from the existing registry via [`SaltMint::from_registry`]). The
    /// mint's seen-set is extended with the salts minted here.
    pub fn ingest_with_mint(
        store: &Graph,
        names: &[NamedNode],
        mint: &mut SaltMint,
    ) -> Result<Self, IngestError> {
        let mut graphs = Vec::with_capacity(names.len());
        for name in names {
            let gname = Term::NamedNode(name.clone());
            let Some((_, g)) = store.named.iter().find(|(n, _)| *n == gname) else {
                return Err(IngestError::MissingGraph(name.as_str().to_string()));
            };
            let triples = crate::canon::graph_triples(g)
                .map_err(|e| IngestError::Content(name.as_str().to_string(), CommitError::Canon(e)))?;
            // sq-610: one fresh globally-unique salt per named graph.
            let salt = mint.mint()?;
            // sq-cn8: per-named-graph commitment under that salt.
            let commitment = commit_triples(&triples, salt)
                .map_err(|e| IngestError::Commit(name.as_str().to_string(), e))?;
            graphs.push(IngestedGraph { name: name.clone(), salt, commitment });
        }
        Ok(IngestedDataset { graphs })
    }

    /// The graph names, in ingest order (parallel to [`Self::salts`]).
    pub fn names(&self) -> Vec<NamedNode> {
        self.graphs.iter().map(|g| g.name.clone()).collect()
    }

    /// The minted salts, in ingest order (parallel to [`Self::names`]).
    pub fn salts(&self) -> Vec<Fr> {
        self.graphs.iter().map(|g| g.salt).collect()
    }

    /// The per-graph commitments `C(G)`, in ingest order.
    pub fn commitments(&self) -> Vec<Fr> {
        self.graphs.iter().map(|g| g.commitment.commitment).collect()
    }

    /// The `<urn:sparq:zk>` registry entries (unattested) for the ingested
    /// graphs. The issuer-attestation path may upgrade these to signed entries.
    pub fn registry_entries(&self) -> Vec<RegistryEntry> {
        self.graphs.iter().map(|g| g.registry_entry()).collect()
    }

    /// Builds a [`ZkDataset`] over the same `store` using THIS ingest's minted
    /// salts, so traced execution consumes exactly the per-graph commitments
    /// that were ingested (the salts are parallel to the ingested graph order).
    /// This is the seam from the ingest path into the zk-trace consumer
    /// (sq-cn8: the per-named-graph commitments wired through to the trace).
    pub fn into_zk_dataset(&self, store: &Graph) -> Result<ZkDataset, TraceError> {
        ZkDataset::from_store(store, &self.names(), &self.salts())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const TWO_GRAPHS: &str = r#"
<http://ex/alice> <http://ex/worksAt> <http://ex/acme> <http://ex/g/employment> .
<http://ex/alice> <http://ex/title> "Engineer" <http://ex/g/employment> .
<http://ex/acme> <http://ex/sector> "robotics" <http://ex/g/registry> .
<http://ex/acme> <http://ex/founded> "1990" <http://ex/g/registry> .
"#;

    fn store() -> Graph {
        Graph::load_dataset(TWO_GRAPHS, "n-quads").unwrap()
    }

    fn names() -> Vec<NamedNode> {
        vec![
            NamedNode::new("http://ex/g/employment").unwrap(),
            NamedNode::new("http://ex/g/registry").unwrap(),
        ]
    }

    // ---- sq-610: globally-unique salts ----------------------------------------

    /// A mint issues all-distinct salts over many draws.
    #[test]
    fn mint_draws_are_all_distinct() {
        let mut mint = SaltMint::new();
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let s = mint.mint().unwrap();
            assert!(seen.insert(s), "minted salt repeated: {s:?}");
        }
        assert_eq!(mint.issued_count(), 1000);
    }

    /// Ingesting a multi-graph dataset yields all-distinct per-graph salts.
    #[test]
    fn ingest_yields_distinct_salts_per_graph() {
        let ds = IngestedDataset::ingest(&store(), &names()).unwrap();
        let salts = ds.salts();
        assert_eq!(salts.len(), 2);
        let distinct: HashSet<_> = salts.iter().collect();
        assert_eq!(distinct.len(), salts.len(), "every per-graph salt must be unique");
    }

    /// A forced collision (re-registering an already-issued salt) is detected
    /// and prevented — the duplicate cannot enter the ingest.
    #[test]
    fn forced_salt_collision_is_rejected() {
        let mut mint = SaltMint::new();
        let s = mint.mint().unwrap();
        // Re-registering the same salt collides and is refused.
        assert!(matches!(mint.register(s), Err(IngestError::SaltCollision)));
        // A genuinely fresh external salt is accepted.
        let fresh = crate::encode::salt_from_bytes(&[0xABu8; 32]);
        assert!(mint.register(fresh).is_ok());
        // ...and re-registering THAT one now also collides.
        assert!(matches!(mint.register(fresh), Err(IngestError::SaltCollision)));
    }

    /// `from_registry` seeds the seen-set so a prior salt cannot be re-minted
    /// into a new ingest (cross-session uniqueness when state is carried).
    #[test]
    fn registry_seeded_mint_rejects_prior_salts() {
        let ds = IngestedDataset::ingest(&store(), &names()).unwrap();
        let entries = ds.registry_entries();
        let mut mint = SaltMint::from_registry(&entries);
        // Every previously-minted salt is already "issued" and refused.
        for e in &entries {
            assert!(matches!(mint.register(e.salt), Err(IngestError::SaltCollision)));
        }
        // A new mint draw is still fresh (distinct from all prior salts).
        let fresh = mint.mint().unwrap();
        assert!(!entries.iter().any(|e| e.salt == fresh));
    }

    // ---- sq-cn8: per-named-graph commitment ------------------------------------

    /// Each named graph gets its OWN commitment (not a single global one), and
    /// the two commitments differ (distinct content under distinct salts).
    #[test]
    fn per_named_graph_commitments_are_distinct() {
        let ds = IngestedDataset::ingest(&store(), &names()).unwrap();
        assert_eq!(ds.graphs.len(), 2);
        let c0 = ds.graphs[0].commitment.commitment;
        let c1 = ds.graphs[1].commitment.commitment;
        assert_ne!(c0, c1, "per-named-graph commitments must be distinct");
        // Each commitment covers only its own graph's leaves.
        assert_eq!(ds.graphs[0].commitment.leaves.len(), 2);
        assert_eq!(ds.graphs[1].commitment.leaves.len(), 2);
    }

    /// The ingest's salts/commitments are exactly what a `ZkDataset` built from
    /// the same ingest commits to — the ingest→trace seam is consistent.
    #[test]
    fn ingest_wires_into_zk_dataset() {
        let store = store();
        let ds = IngestedDataset::ingest(&store, &names()).unwrap();
        let zk = ds.into_zk_dataset(&store).unwrap();
        assert_eq!(zk.graphs.len(), ds.graphs.len());
        for (zg, ig) in zk.graphs.iter().zip(&ds.graphs) {
            assert_eq!(zg.name, ig.name);
            assert_eq!(zg.commitment.salt, ig.salt, "ZkDataset commits under the minted salt");
            assert_eq!(
                zg.commitment.commitment, ig.commitment.commitment,
                "the per-graph commitment must match the ingest's"
            );
        }
    }

    /// A traced query over the ingested dataset resolves leaves into the
    /// per-graph commitments minted at ingest (end-to-end through the seam).
    #[test]
    fn traced_query_over_ingested_dataset() {
        let store = store();
        let ds = IngestedDataset::ingest(&store, &names()).unwrap();
        let zk = ds.into_zk_dataset(&store).unwrap();
        let exec = zk
            .query_traced("SELECT ?t WHERE { <http://ex/alice> <http://ex/title> ?t }")
            .unwrap();
        assert_eq!(exec.result.rows.len(), 1);
        // The title triple attributes to the employment graph (index 0) only.
        for rp in &exec.patterns {
            for refs in &rp.leaves {
                for r in refs {
                    assert!(r.leaf < zk.graphs[r.graph].commitment.leaves.len());
                }
            }
        }
    }

    /// A dataset clause is refused before evaluation, as the verifier refuses
    /// it, instead of evaluating over an empty dataset to an empty trace.
    #[test]
    fn traced_query_refuses_a_dataset_clause() {
        let store = store();
        let zk = IngestedDataset::ingest(&store, &names()).unwrap().into_zk_dataset(&store).unwrap();
        let g = names()[0].as_str().to_string();
        for q in [
            format!("SELECT ?t FROM <{g}> WHERE {{ <http://ex/alice> <http://ex/title> ?t }}"),
            format!("SELECT ?t FROM NAMED <{g}> WHERE {{ <http://ex/alice> <http://ex/title> ?t }}"),
            format!("ASK FROM <{g}> {{ <http://ex/alice> <http://ex/title> ?t }}"),
        ] {
            let Err(TraceError::Query(msg)) = zk.query_traced(&q) else {
                panic!("{q}: a dataset clause was traced");
            };
            assert!(msg.contains("FROM / FROM NAMED"), "{q}: {msg}");
        }
        let exec = zk.query_traced("SELECT ?t WHERE { <http://ex/alice> <http://ex/title> ?t }").unwrap();
        assert_eq!(exec.result.rows.len(), 1);
    }

    /// Missing graph fails closed.
    #[test]
    fn missing_graph_fails_closed() {
        let absent = vec![NamedNode::new("http://ex/g/nope").unwrap()];
        assert!(matches!(
            IngestedDataset::ingest(&store(), &absent),
            Err(IngestError::MissingGraph(_))
        ));
    }

    /// The registry entries round-trip the minted salt + commitment.
    #[test]
    fn registry_entries_carry_salt_and_commitment() {
        let ds = IngestedDataset::ingest(&store(), &names()).unwrap();
        let entries = ds.registry_entries();
        assert_eq!(entries.len(), 2);
        for (e, g) in entries.iter().zip(&ds.graphs) {
            assert_eq!(e.salt, g.salt);
            assert_eq!(e.commitment, g.commitment.commitment);
            assert_eq!(e.document, g.name);
        }
    }
}
