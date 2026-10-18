//! `V("phrase")` lexical-first concept resolution behind the soundness envelope
//! (design §3.3 / §6, Phase 2 — the top recommendation of research #1074).
//!
//! `V("phrase")` lets an agent write the *concept it means* instead of guessing an opaque
//! content-addressed PKG IRI it cannot know a priori — attacking **grounding**, the
//! measured first-shot bottleneck in the text-to-SPARQL literature (design §1.1), rather
//! than syntax. It is parse-time sugar: the transpiler resolves `phrase` to a concept IRI
//! and splices a canonical `<iri>` into the emitted SPARQL, so the engine only ever sees
//! standard SPARQL (design §3.3).
//!
//! The resolution is **sound only behind the §6 envelope**, all of which is enforced here:
//!
//! 1. **Lexical-first, vector-fallback** (§6.4). The deterministic, no-model, no-network
//!    `sparq-nlq` lexical linker is the PRIMARY path; the vector fallback fires only when
//!    lexical returns nothing. Each [`Resolution`] records which [`Method`] fired.
//! 2. **Echo IRI + score + runner-up + confidence** (§6.2). Every binding returns a
//!    [`Resolution`] the agent can verify; a close runner-up is a visible ambiguity flag.
//! 3. **Confidence-gated, never auto-accept the uncertain** (§6.3). Below the confidence
//!    floor, or when the top score and runner-up are within the ambiguity margin, `V()`
//!    does **not** bind — it returns [`crate::TerseError::Unresolved`] with the candidate
//!    list. Loud-fail beats silent-wrong.
//! 4. **Mandatory staleness guard** (§6.5). The vector fallback verifies the store's
//!    fingerprint against the graph generation (`VectorStore::check_graph`); a store built
//!    against a different graph generation is a hard [`crate::TerseError::StaleStore`],
//!    never stale neighbours.
//!
//! The vector fallback needs the query *phrase* embedded into a vector. Embedding free text
//! requires a model — an explicit opt-in network/dependency cost the design (§3.3, §9 Q5)
//! flags. This crate does **not** force an embedder into the build: the caller supplies the
//! pre-embedded query vector (e.g. via `sparq-nlq`'s `live` path or a local model), keeping
//! this crate lean and the model dependency the caller's explicit choice. With no embedded
//! vector supplied, resolution is **lexical-only** — the no-model, deterministic default.

/// Which resolution method bound a `V("phrase")` (design §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Deterministic exact/substring label match via the `sparq-nlq` lexical linker (no
    /// model, no network). The preferred, drift-free path.
    Lexical,
    /// Vector nearest-neighbour over a pre-embedded query phrase, staleness-guarded. Fires
    /// only when lexical returns nothing.
    Vector,
}

impl Method {
    /// A stable lowercase tag for diagnostics / serialisation.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Lexical => "lexical",
            Method::Vector => "vector",
        }
    }
}

/// The verifiable record of one `V("phrase")` resolution (design §6.2). Returned in
/// [`crate::Expansion::resolutions`] so the agent can audit every bind: the resolved IRI,
/// its score, the runner-up (a close one flags ambiguity), the confidence, and which
/// [`Method`] fired.
///
/// This type is **always present** (so the public API compiles in both feature states),
/// but it is only ever *constructed* behind the `vectors` feature.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    /// The phrase from the `V("phrase")` construct.
    pub phrase: String,
    /// The resolved concept IRI, as a string (kept feature-independent; the `vectors`
    /// build validates it is a real IRI before binding).
    pub iri: String,
    /// The top candidate's score (lexical: a label-predicate rank; vector: cosine in
    /// `[-1, 1]`). Higher is better.
    pub score: f32,
    /// The second-best candidate's IRI, if any (the ambiguity flag).
    pub runner_up: Option<String>,
    /// The runner-up's score, if any.
    pub runner_up_score: Option<f32>,
    /// A `[0, 1]` confidence: how clearly the top candidate beats the runner-up (the
    /// margin), so the agent can gate on a single number. `1.0` when there is no runner-up.
    pub confidence: f32,
    /// Which method fired (design §6.4).
    pub method: Method,
}

#[cfg(feature = "vectors")]
pub use ctx::{ResolveCtx, ResolveGate};

#[cfg(feature = "vectors")]
mod ctx {
    use super::{Method, Resolution};
    use crate::error::TerseError;
    use oxrdf::{NamedNode, Term};
    use sparq_core::Graph;
    use sparq_nlq::link::EntityLinker;
    use sparq_vectors::store::VectorStore;

    /// The confidence/ambiguity gate for `V()` (design §6.3). A resolution binds only if it
    /// clears the floor AND its margin over the runner-up clears the ambiguity bar.
    #[derive(Debug, Clone, Copy)]
    pub struct ResolveGate {
        /// Minimum top-candidate score to bind at all. A vector score is cosine in
        /// `[-1, 1]`; the lexical linker emits label-predicate ranks (`>= 1.0`), so this
        /// floor is interpreted per-method.
        pub min_score: f32,
        /// Minimum confidence (top-vs-runner-up margin, `[0, 1]`) to bind. When the top and
        /// runner-up are within this margin the bind is *ambiguous* and is refused.
        pub min_confidence: f32,
    }

    impl Default for ResolveGate {
        /// Conservative defaults: a positive vector floor and a non-trivial margin. These
        /// are pre-registration knobs (design §9 Q2) — a caller tunes them per corpus; the
        /// defaults err toward refusing (loud-fail) rather than guessing.
        fn default() -> Self {
            ResolveGate {
                min_score: 0.25,
                min_confidence: 0.10,
            }
        }
    }

    /// A reusable, graph-bound resolution context for `V("phrase")` (design §3.3 / §6).
    /// Built once over a [`Graph`]; resolves any number of phrases lexical-first, with an
    /// optional staleness-guarded vector fallback when a [`VectorStore`] is attached.
    ///
    /// The lexical linker is constructed once (one scan per label predicate) and reused;
    /// the vector fallback is engaged per-phrase by supplying a pre-embedded query vector
    /// (this crate never embeds free text itself — the model is the caller's explicit,
    /// opt-in dependency; design §6/§9 Q5).
    pub struct ResolveCtx<'g> {
        graph: &'g Graph,
        linker: EntityLinker<'g>,
        store: Option<&'g VectorStore>,
        gate: ResolveGate,
    }

    impl<'g> ResolveCtx<'g> {
        /// Builds a lexical-only resolution context over `graph` (the no-model,
        /// deterministic default). `expand_k = 0` and a generous `max_links` keep the linker
        /// focused on direct label matches for single-phrase resolution.
        pub fn lexical(graph: &'g Graph) -> Self {
            ResolveCtx {
                graph,
                linker: EntityLinker::build(graph, 0, 16),
                store: None,
                gate: ResolveGate::default(),
            }
        }

        /// Attaches a [`VectorStore`] so phrases that fail lexical linking can fall back to
        /// the staleness-guarded vector search (design §6.4/§6.5). The store must be keyed
        /// to `graph`'s generation — a mismatch is caught per-resolution as
        /// [`TerseError::StaleStore`].
        pub fn with_vector_store(mut self, store: &'g VectorStore) -> Self {
            self.store = Some(store);
            self
        }

        /// Overrides the confidence/ambiguity gate (design §6.3).
        pub fn with_gate(mut self, gate: ResolveGate) -> Self {
            self.gate = gate;
            self
        }

        /// The active gate.
        pub fn gate(&self) -> ResolveGate {
            self.gate
        }

        /// Resolves one `phrase` to a concept IRI, lexical-first (design §6.4), applying the
        /// §6 envelope. On success the returned [`Resolution`] echoes the IRI + score +
        /// runner-up + confidence + method; on failure the error is the loud, agent-visible
        /// signal (never a silent guess):
        ///
        /// - [`TerseError::Unresolved`] — no candidates, below the score floor, or ambiguous
        ///   (the candidate list is attached for disambiguation; design §6.3).
        /// - [`TerseError::StaleStore`] — the vector fallback's store is stale vs the graph
        ///   (design §6.5).
        ///
        /// `query_vec` is the pre-embedded query phrase used *only* for the vector fallback
        /// (which fires only when lexical returns nothing). Pass `None` for the lexical-only,
        /// no-model path.
        pub fn resolve(
            &self,
            phrase: &str,
            query_vec: Option<&[f32]>,
        ) -> Result<Resolution, TerseError> {
            self.resolve_lazy(phrase, |_| query_vec.map(<[f32]>::to_vec))
        }

        /// [`Self::resolve`] with the query vector produced *on demand*: `embed` is invoked
        /// only if the lexical path returns nothing AND a *usable* [`VectorStore`] is attached
        /// — attached and fresh vs the graph (§6.5) — i.e. only when the vector fallback can
        /// actually fire. Embedding free text is a model / network cost the design keeps the
        /// caller's explicit opt-in (§6/§9 Q5), so a phrase the deterministic lexical linker
        /// already binds must never pay it — this is the entry point
        /// [`crate::terse_to_sparql_with`] uses so its per-phrase embedder contract ("called
        /// only for a phrase that fails lexical linking") holds.
        ///
        /// Because the staleness guard runs *before* `embed`, a stale store is
        /// [`TerseError::StaleStore`] regardless of what the embedder would have returned —
        /// the loud, accurate error, and no phrase is disclosed to the model.
        pub(crate) fn resolve_lazy(
            &self,
            phrase: &str,
            embed: impl FnOnce(&str) -> Option<Vec<f32>>,
        ) -> Result<Resolution, TerseError> {
            // ---- §6.4: lexical-first — no model, no network, no embedder call. ----
            if let Some(res) = self.resolve_lexical(phrase) {
                return self.gate_resolution(phrase, res);
            }
            // ---- §6.4: vector fallback. Only NOW is the embedder worth its cost, and only
            // when a store that can actually serve the search is attached. ----
            if let Some(store) = self.store {
                // §6.5 FIRST: a store that cannot serve the fallback must not cost the caller
                // an embedding (a model/network round-trip that also discloses the phrase).
                self.check_store_fresh(phrase, store)?;
                if let Some(qv) = embed(phrase) {
                    return self.resolve_vector(phrase, store, &qv);
                }
            }
            // Nothing matched lexically and no usable vector fallback: loud failure.
            Err(TerseError::Unresolved {
                phrase: phrase.to_string(),
                why: if self.store.is_some() {
                    "no lexical match and no embedded query vector supplied for the vector \
                     fallback"
                        .to_string()
                } else {
                    "no lexical match (lexical-only context — attach a vector store + \
                     supply an embedded query for the fuzzy fallback)"
                        .to_string()
                },
                candidates: Vec::new(),
            })
        }

        /// The deterministic lexical path: the top-two label matches for `phrase` via the
        /// `sparq-nlq` linker, as a raw (pre-gate) [`Resolution`]. `None` if nothing linked.
        fn resolve_lexical(&self, phrase: &str) -> Option<Resolution> {
            let linking = self.linker.link(phrase);
            if linking.entities.is_empty() {
                return None;
            }
            // The linker already ranks entities (exact > substring, then label-predicate
            // score). Map its rank into a comparable score: an exact full-label match is the
            // strongest possible lexical signal (1.0), a substring match weaker (0.6).
            let lexical_score = |exact: bool| if exact { 1.0f32 } else { 0.6f32 };
            let top = &linking.entities[0];
            let runner = linking.entities.get(1);
            Some(Resolution {
                phrase: phrase.to_string(),
                iri: top.iri.as_str().to_string(),
                score: lexical_score(top.exact),
                runner_up: runner.map(|r| r.iri.as_str().to_string()),
                runner_up_score: runner.map(|r| lexical_score(r.exact)),
                confidence: 0.0, // filled by gate_resolution
                method: Method::Lexical,
            })
        }

        /// The §6.5 staleness guard, in one place: a store built against a different graph
        /// generation is a hard [`TerseError::StaleStore`], never stale neighbours. Probing
        /// via `check_graph` surfaces a mismatch even though the brute-force search takes a
        /// raw vector (not a term).
        ///
        /// [`Self::resolve_lazy`] runs this *before* the embedder, so the fallback's single
        /// call site enters [`Self::resolve_vector`] already validated. It is deliberately
        /// not repeated there: `check_graph` recomputes the graph fingerprint (a scan of the
        /// dictionary), so a second per-phrase call would be a real, redundant cost.
        fn check_store_fresh(&self, phrase: &str, store: &VectorStore) -> Result<(), TerseError> {
            store
                .check_graph(self.graph)
                .map_err(|detail| TerseError::StaleStore {
                    phrase: phrase.to_string(),
                    detail,
                })
        }

        /// The vector fallback (design §6.4/§6.5). Embeds-via-caller: the supplied `query_vec`
        /// is searched against `store` over the graph; the nearest term is the candidate.
        ///
        /// **Precondition:** `store` has already passed [`Self::check_store_fresh`] — the
        /// mandatory §6.5 guard, hoisted into [`Self::resolve_lazy`] ahead of the embedder.
        fn resolve_vector(
            &self,
            phrase: &str,
            store: &VectorStore,
            query_vec: &[f32],
        ) -> Result<Resolution, TerseError> {
            // Top-2 nearest by cosine; map ids back to IRIs (skip non-NamedNode terms).
            let hits = sparq_vectors::ann::nearest_exact(store, query_vec, 8);
            let mut candidates: Vec<(NamedNode, f32)> = Vec::new();
            for (id, score) in hits {
                if let Term::NamedNode(n) = self.graph.dict().term(id) {
                    candidates.push((n, score));
                    if candidates.len() == 2 {
                        break;
                    }
                }
            }
            if candidates.is_empty() {
                return Err(TerseError::Unresolved {
                    phrase: phrase.to_string(),
                    why: "vector fallback found no concept (NamedNode) neighbours".to_string(),
                    candidates: Vec::new(),
                });
            }
            let (top_iri, top_score) = candidates[0].clone();
            let runner = candidates.get(1).cloned();
            let res = Resolution {
                phrase: phrase.to_string(),
                iri: top_iri.as_str().to_string(),
                score: top_score,
                runner_up: runner.as_ref().map(|(n, _)| n.as_str().to_string()),
                runner_up_score: runner.as_ref().map(|&(_, s)| s),
                confidence: 0.0, // filled by gate_resolution
                method: Method::Vector,
            };
            self.gate_resolution(phrase, res)
        }

        /// Applies the §6.3 confidence/ambiguity gate to a raw resolution: computes the
        /// confidence (the normalised top-vs-runner-up margin), refuses below the score
        /// floor or inside the ambiguity margin, and otherwise validates the IRI and returns
        /// the gated [`Resolution`].
        fn gate_resolution(
            &self,
            phrase: &str,
            mut res: Resolution,
        ) -> Result<Resolution, TerseError> {
            let confidence = confidence_of(res.score, res.runner_up_score);
            res.confidence = confidence;

            // §6.3: below the score floor -> do not bind.
            if res.score < self.gate.min_score {
                return Err(TerseError::Unresolved {
                    phrase: phrase.to_string(),
                    why: format!(
                        "top score {:.3} below the floor {:.3}",
                        res.score, self.gate.min_score
                    ),
                    candidates: candidate_pairs(&res),
                });
            }
            // §6.3: ambiguous (top and runner-up too close) -> do not bind.
            if confidence < self.gate.min_confidence {
                return Err(TerseError::Unresolved {
                    phrase: phrase.to_string(),
                    why: format!(
                        "ambiguous: confidence {:.3} below the margin {:.3} (top and \
                         runner-up too close)",
                        confidence, self.gate.min_confidence
                    ),
                    candidates: candidate_pairs(&res),
                });
            }
            // Validate the resolved IRI is a real IRI before we splice it into SPARQL — a
            // malformed IRI would otherwise fail the downstream canary opaquely.
            if NamedNode::new(&res.iri).is_err() {
                return Err(TerseError::Unresolved {
                    phrase: phrase.to_string(),
                    why: format!("resolved value {} is not a valid IRI", res.iri),
                    candidates: candidate_pairs(&res),
                });
            }
            Ok(res)
        }
    }

    /// The candidate `(iri, score)` pairs of a resolution, top first, for an error's
    /// candidate list.
    fn candidate_pairs(res: &Resolution) -> Vec<(String, f32)> {
        let mut v = vec![(res.iri.clone(), res.score)];
        if let (Some(iri), Some(s)) = (res.runner_up.clone(), res.runner_up_score) {
            v.push((iri, s));
        }
        v
    }

    /// Confidence in `[0, 1]`: the top-vs-runner-up margin normalised by the top score.
    /// `1.0` when there is no runner-up (an unambiguous single match). When scores can be
    /// negative (cosine), the margin is clamped into `[0, 1]`.
    fn confidence_of(top: f32, runner_up: Option<f32>) -> f32 {
        match runner_up {
            None => 1.0,
            Some(r) => {
                let denom = top.abs().max(1e-6);
                ((top - r) / denom).clamp(0.0, 1.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_tags_are_stable() {
        assert_eq!(Method::Lexical.as_str(), "lexical");
        assert_eq!(Method::Vector.as_str(), "vector");
    }

    #[test]
    fn resolution_is_constructible_for_inspection() {
        // The struct is part of the always-present public API (Expansion.resolutions).
        let r = Resolution {
            phrase: "cat".into(),
            iri: "http://ex/cat".into(),
            score: 1.0,
            runner_up: None,
            runner_up_score: None,
            confidence: 1.0,
            method: Method::Lexical,
        };
        assert_eq!(r.method, Method::Lexical);
        assert_eq!(r.iri, "http://ex/cat");
    }

    /// End-to-end `V()` resolution (`sq-26fdp`): a verbatim `skos:prefLabel` that shares a
    /// token with a sibling concept must bind unambiguously at the strongest lexical signal,
    /// not loud-fail as ambiguous. Exercises the REAL `ResolveCtx::resolve` lexical path.
    #[cfg(feature = "vectors")]
    mod verbatim_preflabel {
        use crate::{Method, ResolveCtx};
        use sparq_core::Graph;

        fn graph() -> Graph {
            // Two SKOS concepts sharing the token "discipline"; one prefLabel carries
            // punctuation (`/`, `+`) — the exact bug-report shape. Two extra concepts
            // (`alpha thing` / `beta gadget`) give a phrase that substring-matches two
            // DIFFERENT concepts via two different tokens, i.e. a genuinely ambiguous fuzzy
            // phrase the gate must still loud-fail on.
            let ttl = r#"
                @prefix kb: <https://sparq.dev/ns/pkg/kb#> .
                @prefix skos: <http://www.w3.org/2004/02/skos/core#> .
                kb:topic-merge-discipline a skos:Concept ; skos:prefLabel "Merge discipline" .
                kb:topic-zk-discipline a skos:Concept ;
                    skos:prefLabel "ZK/MPC claim + circuit discipline" .
                kb:topic-alpha a skos:Concept ; skos:prefLabel "alpha thing" .
                kb:topic-beta a skos:Concept ; skos:prefLabel "beta gadget" .
            "#;
            Graph::load_str(ttl, "turtle").expect("graph parses")
        }

        #[test]
        fn verbatim_punctuated_preflabel_binds_at_full_confidence() {
            let g = graph();
            let ctx = ResolveCtx::lexical(&g);
            // Before the fix this loud-failed AMBIGUOUS on the shared "discipline" token.
            let res = ctx
                .resolve("ZK/MPC claim + circuit discipline", None)
                .expect("a verbatim prefLabel must resolve, not loud-fail");
            assert_eq!(res.iri, "https://sparq.dev/ns/pkg/kb#topic-zk-discipline");
            assert_eq!(res.method, Method::Lexical);
            assert_eq!(res.score, 1.0, "an exact full-label hit is the strongest signal");
            assert!(res.runner_up.is_none(), "the verbatim hit is the sole candidate");
            assert_eq!(res.confidence, 1.0, "no runner-up => full confidence");
        }

        #[test]
        fn sibling_verbatim_preflabel_still_resolves_to_its_own_concept() {
            // The pre-existing working case stays working: "Merge discipline" -> its topic.
            let g = graph();
            let ctx = ResolveCtx::lexical(&g);
            let res = ctx.resolve("Merge discipline", None).expect("resolves");
            assert_eq!(res.iri, "https://sparq.dev/ns/pkg/kb#topic-merge-discipline");
            assert_eq!(res.score, 1.0);
            assert!(res.runner_up.is_none());
        }

        #[test]
        fn genuinely_ambiguous_fuzzy_phrase_still_loud_fails() {
            // The fix must NOT weaken the loud-fail: a phrase that is not a whole label and
            // substring-matches two DIFFERENT concepts at the same score is still ambiguous.
            // "alpha beta" is not a label; "alpha" -> topic-alpha, "beta" -> topic-beta, both
            // non-exact (0.6) => a tie the gate must refuse rather than guess.
            let g = graph();
            let ctx = ResolveCtx::lexical(&g);
            let err = ctx
                .resolve("alpha beta", None)
                .expect_err("a tie across two concepts must loud-fail, never guess");
            assert!(
                matches!(err, crate::TerseError::Unresolved { .. }),
                "expected Unresolved, got {err:?}"
            );
        }
    }
}
