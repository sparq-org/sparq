//! # `framework_vocab` — the `trustx:` certification-scope vocabulary (Rust constants)
//!
//! The IRIs of the trust-expression program's **certification-scope layer** as Rust
//! constants: the terms that let a verifier express *"certified issuers WITHIN the
//! eIDAS / DIATF framework, that have only issued information they are certified to
//! issue"* (issue #1592, decomposition record `research/trust-expression-spec.md` §3.4
//! / D4). This module **extends** the [`crate::vocab`] `trust:` vocabulary (it shares
//! the same placeholder base IRI, `https://sparq.dev/ns/trust#`, since `trustx:` is a
//! prose-only sub-prefix of `trust:` — not a distinct namespace) and **references** the
//! vendored `sec-req:` regulatory-framework individuals (eIDAS 2.0 / UK DVS) rather than
//! duplicating them. It does **not** fork `trust:` and redeclares no `trust:` or
//! `sec-req:` term (the design's D5 no-duplication rule).
//!
//! The three surfaces the layer adds over what the estate already had:
//!
//! - **The trust requirements** (`TRUSTX_TRUST_REQUIREMENTS` and its properties) — the
//!   verifier→holder contract carrier: ONE small RDF graph binding a query to its trust
//!   conditions (design §3.1 / D1). There is no new query syntax; the trust conditions
//!   live in the requirements graph, never in the query.
//! - **The two trust modes** — enumerated parties (`TRUSTX_TRUSTS_ISSUER`) and
//!   framework-certified issuers (`TRUSTX_TRUSTS_FRAMEWORK`), composing by plain OR
//!   (design §3.2 / D2).
//! - **The certification-scope terms** (`TRUSTX_CERTIFICATION`, `TRUSTX_CERTIFICATION_SCOPE`, …)
//!   — an issuer being *certified under* a framework, over a *scope* that ranges from
//!   service-level (`TRUSTX_ANY_SERVICE_SCOPE`, the honest DIATF granularity) down to
//!   a predicate set / SHACL shape reusing the existing `trust:forShape` idiom
//!   (design §3.4 / D4).
//!
//! ## What this vocabulary is — and is NOT (honesty, load-bearing)
//!
//! Framework-anchored trust is **anchored, not proven**. A `TRUSTX_CERTIFICATION`
//! bottoms out in the framework operator's signed trusted-list / register artifacts —
//! it is a *trust anchor*, not cryptography. The scope-conformance check constrains what
//! the *verifier accepts* and, via the published certification, what the issuer was
//! *authorised* to issue; it **cannot** retroactively prove an issuer never mis-issued
//! elsewhere (design §7.2). No term here asserts a settled cryptographic soundness or
//! privacy guarantee: the sparq ZK estate is internally re-audited but has **no external
//! accredited-cryptographer sign-off** (open gate `sq-qhy4`), and `sparq-mpc` is
//! honest-majority **semi-honest only**. Non-revocation is a *positive*, time-windowed
//! status attestation (`TRUSTX_STATUS_ATTESTATION`) — existence of a covering window,
//! never evidence-of-absence (OWA/monotonicity; design §3.3 / D3).
//!
//! ## Opt-in by construction
//!
//! This whole module (and the `trust-framework.ttl` it pins) is behind the **default-OFF
//! `framework-vocab`** cargo feature, so it adds **nothing** to the lean default build —
//! it is plain `const &str` data plus a test, no new dependency, strictly additive
//! (the same discipline as [`crate::secprop`]).
//!
//! The canonical machine-readable form is
//! `ontologies/trust/trust-framework.ttl`; the `framework_ttl_iris_match_rust_constants`
//! test below keeps the published Turtle and these constants from drifting (the same
//! discipline as [`crate::vocab`]).
//!
//! [FABLE-5] sq-6syab.2 (epic sq-6syab; issue #1592; design record
//! `research/trust-expression-spec.md`). 🤖 SPARQ agent — trust-expression
//! certification-scope layer.

/// The `trust:` / `trustx:` namespace base (non-standard; a WG would rehome it). Shared
/// with [`crate::vocab::TRUST_NS`] — `trustx:` extends `trust:` under the SAME base IRI.
pub const TRUSTX_NS: &str = "https://sparq.dev/ns/trust#";

/// The vendored `sec-req:` namespace (a real `w3id.org` permanent identifier). The
/// framework individuals below `rdfs:seeAlso` its eIDAS / UK DVS requirement instances;
/// this constant lets the sync test recognise those references as EXEMPT from the
/// `trustx:`-declaration drift check (they are referenced, never redeclared).
pub const SEC_REQ_NS: &str = "https://w3id.org/zkp-sparql/sec-req#";

// ── the trust requirements — the verifier→holder contract carrier (design §3.1) ──

/// `trustx:TrustRequirements` — the verifier→holder contract carrier: a small RDF graph
/// binding a query to its trust conditions. The trust conditions live HERE, never in
/// the query (no new query syntax — design §3.1 / D1).
pub const TRUSTX_TRUST_REQUIREMENTS: &str = "https://sparq.dev/ns/trust#TrustRequirements";
/// `trustx:question` — an opaque IRI naming the question a trust-requirements
/// document was authored for. A correlation label ONLY: no layer of this crate
/// resolves or verifies it against the query string, so the question↔query
/// association belongs to whoever authenticates the request (see the
/// `expression` module's honest-scope docs).
pub const TRUSTX_QUESTION: &str = "https://sparq.dev/ns/trust#question";
/// `trustx:trustsIssuer` — **MODE 1** (enumerated parties): admit contributing
/// statements issued by this enumerated issuer identity (a DID/key binding; the
/// maintainer's `[X, Y and Z]` set). Composes with framework mode by plain OR.
pub const TRUSTX_TRUSTS_ISSUER: &str = "https://sparq.dev/ns/trust#trustsIssuer";
/// `trustx:trustsFramework` — **MODE 2** (framework-certified issuers): admit statements
/// from issuers certified under this [`TRUSTX_FRAMEWORK`] (subject to scope conformance).
pub const TRUSTX_TRUSTS_FRAMEWORK: &str = "https://sparq.dev/ns/trust#trustsFramework";
/// `trustx:requiresScopeConformance` — when true (mode 2), require every contributing
/// statement's type/predicate to fall under its issuer's certification scope valid at
/// the evaluation time (the "only issued what certified to issue" check — design §3.4).
pub const TRUSTX_REQUIRES_SCOPE_CONFORMANCE: &str =
    "https://sparq.dev/ns/trust#requiresScopeConformance";
/// `trustx:requiresValidStatusAt` — the instant *t* at which a covering positive
/// [`TRUSTX_STATUS_ATTESTATION`] must hold. A verifier-chosen parameter, not a constant.
pub const TRUSTX_REQUIRES_VALID_STATUS_AT: &str =
    "https://sparq.dev/ns/trust#requiresValidStatusAt";
/// `trustx:methodPolicy` — OPTIONAL reference to an ODRL policy consumed by the existing
/// secprop/ODRL `admissible()` pre-check (which SOURCES vs which PROOF METHODS is an
/// orthogonal axis; the spec references, not restates, that machinery — design §3.5).
pub const TRUSTX_METHOD_POLICY: &str = "https://sparq.dev/ns/trust#methodPolicy";

// ── the certification-scope layer (design §3.4 / D4) ─────────────────────────

/// `trustx:Framework` — a governance framework operating a trusted-list / register
/// (eIDAS 2.0's Trusted Lists, the UK DIATF register). The individuals
/// [`TRUSTX_EIDAS2`] / [`TRUSTX_DIATF`] `rdfs:seeAlso` the vendored `sec-req:` ones.
pub const TRUSTX_FRAMEWORK: &str = "https://sparq.dev/ns/trust#Framework";
/// `trustx:Certification` — a time-windowed, authority-signed attestation that an issuer
/// is certified under a framework. A Trusted-List / DIATF-register entry IS one.
pub const TRUSTX_CERTIFICATION: &str = "https://sparq.dev/ns/trust#Certification";
/// `trustx:certifies` — the issuer identity (DID/key) a certification certifies.
pub const TRUSTX_CERTIFIES: &str = "https://sparq.dev/ns/trust#certifies";
/// `trustx:underFramework` — the [`TRUSTX_FRAMEWORK`] a certification is issued under.
pub const TRUSTX_UNDER_FRAMEWORK: &str = "https://sparq.dev/ns/trust#underFramework";
/// `trustx:certificationScope` — what the issuer is certified to issue: service-level
/// ([`TRUSTX_ANY_SERVICE_SCOPE`], the honest DIATF granularity), an attestation-type /
/// Rulebook IRI, or a predicate set / SHACL shape (reusing `trust:forShape`).
///
/// NAMED `certificationScope`, **not** `scope`: the `trustx:` prefix shares the `trust:`
/// base IRI, so a `trustx:scope` term would be *the same IRI* as
/// [`crate::vocab::SCOPE`] (rule applicability, `rdfs:domain trust:TrustRule`) rather
/// than a distinct homonym — one property carrying two conflicting domains (issue
/// #3801). The certification-scope reading gets its own local name.
pub const TRUSTX_CERTIFICATION_SCOPE: &str = "https://sparq.dev/ns/trust#certificationScope";
/// `trustx:validFrom` — start of a certification's / status attestation's validity
/// window (inclusive). Its `rdfs:domain` is the UNION
/// [`TRUSTX_CERTIFICATION`] ⊔ [`TRUSTX_STATUS_ATTESTATION`], since both classes carry
/// the window; a bare `Certification` domain would entail that every status attestation
/// is a certification (issue #3801).
pub const TRUSTX_VALID_FROM: &str = "https://sparq.dev/ns/trust#validFrom";
/// `trustx:validUntil` — end of a certification's / status attestation's validity
/// window (inclusive). Same union domain as [`TRUSTX_VALID_FROM`].
pub const TRUSTX_VALID_UNTIL: &str = "https://sparq.dev/ns/trust#validUntil";
/// `trustx:AnyServiceScope` — the coarsest, service-level [`TRUSTX_CERTIFICATION_SCOPE`] value
/// ("everything this certified service issues"), the honest DIATF granularity.
pub const TRUSTX_ANY_SERVICE_SCOPE: &str = "https://sparq.dev/ns/trust#AnyServiceScope";

// ── positive, time-windowed status attestation (design §3.3 / D3) ────────────

/// `trustx:StatusAttestation` — a POSITIVE, time-windowed, authority-signed statement
/// that a credential's / certification's status was valid in `[validFrom, validUntil]`.
/// "Unrevoked" is asked as EXISTENCE of a covering attestation, never as absence of a
/// revocation (OWA/monotonicity — the agreed #1592 constraint).
pub const TRUSTX_STATUS_ATTESTATION: &str = "https://sparq.dev/ns/trust#StatusAttestation";
/// `trustx:coveredBy` — links a contributing statement's provenance reifier to the
/// [`TRUSTX_STATUS_ATTESTATION`] that covered it at the evaluation instant.
pub const TRUSTX_COVERED_BY: &str = "https://sparq.dev/ns/trust#coveredBy";

// ── framework individuals — thin, referencing the vendored sec-req: ──────────

/// `trustx:eIDAS2` — the eIDAS 2.0 framework individual; `rdfs:seeAlso`
/// `sec-req:Eidas20` (schema-level scope granularity via Attestation Rulebooks).
pub const TRUSTX_EIDAS2: &str = "https://sparq.dev/ns/trust#eIDAS2";
/// `trustx:DIATF` — the UK DIATF framework individual; `rdfs:seeAlso` `sec-req:UkDvs`
/// (service-level scope granularity — hence [`TRUSTX_ANY_SERVICE_SCOPE`]).
pub const TRUSTX_DIATF: &str = "https://sparq.dev/ns/trust#DIATF";

/// The vendored `sec-req:` regulatory-requirement individual for eIDAS 2.0 — the
/// existing instance [`TRUSTX_EIDAS2`] `rdfs:seeAlso`s (NOT redeclared here; no
/// duplication of the vendored file — design's D5).
pub const SEC_REQ_EIDAS20: &str = "https://w3id.org/zkp-sparql/sec-req#Eidas20";
/// The vendored `sec-req:` regulatory-requirement individual for UK DVS — the existing
/// instance [`TRUSTX_DIATF`] `rdfs:seeAlso`s (NOT redeclared here; no duplication).
pub const SEC_REQ_UK_DVS: &str = "https://w3id.org/zkp-sparql/sec-req#UkDvs";

/// Every `trustx:` IRI this module mints (the certification-scope layer), plus the two
/// vendored `sec-req:` individuals the framework instances reference. The single source
/// of truth the `tests` drift-check iterates — keeping the published
/// `trust-framework.ttl` and these constants from diverging (the [`crate::vocab`] and
/// [`crate::secprop`] discipline).
pub const ALL_TRUSTX_IRIS: &[&str] = &[
    // trust requirements
    TRUSTX_TRUST_REQUIREMENTS,
    TRUSTX_QUESTION,
    TRUSTX_TRUSTS_ISSUER,
    TRUSTX_TRUSTS_FRAMEWORK,
    TRUSTX_REQUIRES_SCOPE_CONFORMANCE,
    TRUSTX_REQUIRES_VALID_STATUS_AT,
    TRUSTX_METHOD_POLICY,
    // certification-scope layer
    TRUSTX_FRAMEWORK,
    TRUSTX_CERTIFICATION,
    TRUSTX_CERTIFIES,
    TRUSTX_UNDER_FRAMEWORK,
    TRUSTX_CERTIFICATION_SCOPE,
    TRUSTX_VALID_FROM,
    TRUSTX_VALID_UNTIL,
    TRUSTX_ANY_SERVICE_SCOPE,
    // status attestation
    TRUSTX_STATUS_ATTESTATION,
    TRUSTX_COVERED_BY,
    // framework individuals (trustx:)
    TRUSTX_EIDAS2,
    TRUSTX_DIATF,
    // referenced vendored sec-req: individuals (NOT redeclared in trust-framework.ttl)
    SEC_REQ_EIDAS20,
    SEC_REQ_UK_DVS,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine-readable `trust-framework.ttl` is the canonical form; keep it pinned
    /// to these constants so the two cannot drift (the [`crate::vocab`] discipline).
    const TTL: &str = include_str!("../ontologies/trust/trust-framework.ttl");
    /// The `trust:` vocabulary this layer extends — read to assert non-duplication.
    const TRUST_TTL: &str = include_str!("../ontologies/trust/trust.ttl");

    /// The two vendored `sec-req:` individuals are REFERENCED (`rdfs:seeAlso`), never
    /// redeclared as `trustx:` subjects, so they are exempt from the declared-subject
    /// check below.
    const SEC_REQ_REFERENCED_ONLY: &[&str] = &[SEC_REQ_EIDAS20, SEC_REQ_UK_DVS];

    /// Every `trustx:` constant lives in the shared `trust:` base namespace; the two
    /// vendored references live in the `sec-req:` namespace.
    #[test]
    fn all_constants_are_in_a_known_namespace() {
        for &iri in ALL_TRUSTX_IRIS {
            assert!(
                iri.starts_with(TRUSTX_NS) || iri.starts_with(SEC_REQ_NS),
                "constant `{}` is not in the trust: or sec-req: namespace",
                iri,
            );
        }
        // The referenced-only set is exactly the sec-req: constants.
        for &iri in SEC_REQ_REFERENCED_ONLY {
            assert!(
                iri.starts_with(SEC_REQ_NS),
                "referenced-only constant `{}` is not in the sec-req: namespace",
                iri,
            );
        }
    }

    /// No duplicate IRI in the registry (a copy-paste guard).
    #[test]
    fn no_duplicate_iris() {
        let mut seen = std::collections::HashSet::new();
        for &iri in ALL_TRUSTX_IRIS {
            assert!(
                seen.insert(iri),
                "duplicate IRI in ALL_TRUSTX_IRIS: {}",
                iri,
            );
        }
    }

    /// Every minted `trustx:` constant is declared in `trust-framework.ttl` as a
    /// `trustx:LocalName` subject (a `trustx:Foo a …` declaration). The vendored
    /// `sec-req:` individuals are referenced (`rdfs:seeAlso`), not subjects of a fresh
    /// declaration, so they are exempt.
    #[test]
    fn framework_ttl_iris_match_rust_constants() {
        for &iri in ALL_TRUSTX_IRIS {
            if SEC_REQ_REFERENCED_ONLY.contains(&iri) {
                continue;
            }
            let local = iri
                .strip_prefix(TRUSTX_NS)
                .expect("every minted constant is in the trust: namespace");
            let decl = format!("trustx:{} a ", local);
            assert!(
                TTL.contains(&decl),
                "trust-framework.ttl is missing a declaration for `trustx:{}` ({}) — \
                 the published vocabulary drifted from the Rust constants (sq-6syab.2)",
                local,
                iri,
            );
        }
    }

    /// And every `trustx:LocalName a` declaration in the Turtle is backed by a Rust
    /// constant — no published term without a constant. Scans start-of-line
    /// `trustx:Foo a …` subjects, the inverse check (the [`crate::vocab`] discipline).
    #[test]
    fn every_ttl_term_has_a_rust_constant() {
        let declared_locals: Vec<&str> = TTL
            .lines()
            .filter_map(|line| {
                let t = line.trim_start();
                let rest = t.strip_prefix("trustx:")?;
                // Skip a bare-namespace metadata line (no local name), if any.
                if rest.starts_with(char::is_whitespace) {
                    return None;
                }
                let name = rest.split_whitespace().next()?;
                // A term declaration line looks like `trustx:Foo a …`.
                if rest[name.len()..].trim_start().starts_with("a ") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !declared_locals.is_empty(),
            "no trustx: term declarations found in trust-framework.ttl — parser/format drift",
        );

        let constant_locals: std::collections::HashSet<&str> = ALL_TRUSTX_IRIS
            .iter()
            .filter_map(|iri| iri.strip_prefix(TRUSTX_NS))
            .collect();

        for local in declared_locals {
            assert!(
                constant_locals.contains(local),
                "trust-framework.ttl declares `trustx:{}` but no Rust constant names it — \
                 add the constant or remove the term (sq-6syab.2)",
                local,
            );
        }
    }

    /// D5 no-duplication: this extension REDECLARES no `trust:` core term. Every
    /// `trustx:LocalName` declared here must be a NEW local name — never one already
    /// declared in `trust.ttl`. There is NO exception: because `trustx:` and `trust:`
    /// share one base IRI, a repeated local name is the SAME property with two
    /// `rdfs:domain` axioms, not a homonym (issue #3801) — which is why the
    /// certification-scope reading is `certificationScope`, not `scope`.
    #[test]
    fn extends_trust_without_redeclaring_its_terms() {
        // The `trust:` core local names (from trust.ttl `trust:Foo a …` declarations).
        let trust_locals: std::collections::HashSet<&str> = TRUST_TTL
            .lines()
            .filter_map(|line| {
                let rest = line.trim_start().strip_prefix("trust:")?;
                if rest.starts_with(char::is_whitespace) {
                    return None;
                }
                let name = rest.split_whitespace().next()?;
                if rest[name.len()..].trim_start().starts_with("a ") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            trust_locals.contains("forShape"),
            "sanity: trust.ttl should declare trust:forShape",
        );

        // Every local name the framework layer mints must be NEW — not already a trust:
        // core term. `scope` in particular: `trust:scope` owns that IRI (issue #3801).
        for &iri in ALL_TRUSTX_IRIS {
            let Some(local) = iri.strip_prefix(TRUSTX_NS) else {
                continue; // sec-req: reference, not a trust: base term
            };
            assert!(
                !trust_locals.contains(local),
                "trustx:{} would REDECLARE a trust: core term — the D5 no-duplication \
                 rule forbids forking trust: (sq-6syab.2)",
                local,
            );
        }
    }

    /// The framework individuals REFERENCE the vendored `sec-req:` individuals via
    /// `rdfs:seeAlso` and do NOT duplicate them: the referenced IRIs appear in the
    /// Turtle as `rdfs:seeAlso` objects, and the vendored `sec-req:` file is never
    /// edited (asserted structurally — the Turtle carries the seeAlso links).
    #[test]
    fn framework_individuals_reference_vendored_sec_req() {
        for &iri in SEC_REQ_REFERENCED_ONLY {
            let local = iri
                .strip_prefix(SEC_REQ_NS)
                .expect("referenced individual is in the sec-req: namespace");
            let see_also = format!("rdfs:seeAlso sec-req:{}", local);
            assert!(
                TTL.contains(&see_also),
                "trust-framework.ttl must rdfs:seeAlso the vendored `sec-req:{}` \
                 individual (no duplication — design D5)",
                local,
            );
            // And it must NOT be redeclared as a subject in this extension file.
            let redecl = format!("sec-req:{} a ", local);
            assert!(
                !TTL.contains(&redecl),
                "trust-framework.ttl must NOT redeclare the vendored `sec-req:{}` \
                 individual (no duplication — design D5)",
                local,
            );
        }
    }

    /// The honesty banner (framework trust is ANCHORED, not proven; the open external
    /// audit gate `sq-qhy4`) is documented in the Turtle — a load-bearing honesty
    /// invariant of the vocabulary (the [`crate::secprop`] audit-gate discipline).
    #[test]
    fn anchored_not_proven_and_audit_gate_are_documented() {
        assert!(
            TTL.contains("sq-qhy4"),
            "trust-framework.ttl must reference the open external-audit gate (sq-qhy4)",
        );
        assert!(
            TTL.contains("ANCHORED, NOT PROVEN") || TTL.contains("anchored, not proven"),
            "trust-framework.ttl must document that framework trust is anchored, not proven",
        );
        assert!(
            TTL.contains("NON-STANDARD"),
            "trust-framework.ttl must carry the NON-STANDARD placeholder-namespace banner",
        );
    }

    // ── one DIRECT unit test per new public constant (coverage-ratchet rule) ──
    // Each asserts the exact IRI a caller depends on (local name under the correct
    // base), so a rename that silently changed a wire-visible IRI is caught here as a
    // direct-coverage failure, not only through the aggregate drift test.

    #[test]
    fn trustx_ns_is_the_shared_trust_base() {
        assert_eq!(TRUSTX_NS, "https://sparq.dev/ns/trust#");
    }

    #[test]
    fn sec_req_ns_is_the_vendored_w3id_base() {
        assert_eq!(SEC_REQ_NS, "https://w3id.org/zkp-sparql/sec-req#");
    }

    #[test]
    fn trust_requirements_iris_are_stable() {
        assert_eq!(
            TRUSTX_TRUST_REQUIREMENTS,
            "https://sparq.dev/ns/trust#TrustRequirements"
        );
        assert_eq!(TRUSTX_QUESTION, "https://sparq.dev/ns/trust#question");
        assert_eq!(TRUSTX_TRUSTS_ISSUER, "https://sparq.dev/ns/trust#trustsIssuer");
        assert_eq!(
            TRUSTX_TRUSTS_FRAMEWORK,
            "https://sparq.dev/ns/trust#trustsFramework"
        );
        assert_eq!(
            TRUSTX_REQUIRES_SCOPE_CONFORMANCE,
            "https://sparq.dev/ns/trust#requiresScopeConformance"
        );
        assert_eq!(
            TRUSTX_REQUIRES_VALID_STATUS_AT,
            "https://sparq.dev/ns/trust#requiresValidStatusAt"
        );
        assert_eq!(TRUSTX_METHOD_POLICY, "https://sparq.dev/ns/trust#methodPolicy");
    }

    #[test]
    fn certification_scope_iris_are_stable() {
        assert_eq!(TRUSTX_FRAMEWORK, "https://sparq.dev/ns/trust#Framework");
        assert_eq!(TRUSTX_CERTIFICATION, "https://sparq.dev/ns/trust#Certification");
        assert_eq!(TRUSTX_CERTIFIES, "https://sparq.dev/ns/trust#certifies");
        assert_eq!(
            TRUSTX_UNDER_FRAMEWORK,
            "https://sparq.dev/ns/trust#underFramework"
        );
        assert_eq!(
            TRUSTX_CERTIFICATION_SCOPE,
            "https://sparq.dev/ns/trust#certificationScope"
        );
        assert_eq!(TRUSTX_VALID_FROM, "https://sparq.dev/ns/trust#validFrom");
        assert_eq!(TRUSTX_VALID_UNTIL, "https://sparq.dev/ns/trust#validUntil");
        assert_eq!(
            TRUSTX_ANY_SERVICE_SCOPE,
            "https://sparq.dev/ns/trust#AnyServiceScope"
        );
    }

    /// The certification-scope property is a DIFFERENT IRI from the `trust:` core
    /// rule-applicability `scope`. Both prefixes resolve to the same base, so this is the
    /// only thing keeping them apart — and if they were equal, one property would carry
    /// `rdfs:domain trust:TrustRule` AND `rdfs:domain trustx:Certification`, typing every
    /// subject of it as both under RDFS entailment (issue #3801).
    #[test]
    fn certification_scope_does_not_collide_with_trust_scope() {
        assert_ne!(
            TRUSTX_CERTIFICATION_SCOPE,
            crate::vocab::SCOPE,
            "trustx: shares trust:'s base IRI — the certification-scope property MUST NOT \
             reuse the `scope` local name (issue #3801)",
        );
    }

    #[test]
    fn status_attestation_iris_are_stable() {
        assert_eq!(
            TRUSTX_STATUS_ATTESTATION,
            "https://sparq.dev/ns/trust#StatusAttestation"
        );
        assert_eq!(TRUSTX_COVERED_BY, "https://sparq.dev/ns/trust#coveredBy");
    }

    #[test]
    fn framework_individual_iris_are_stable() {
        assert_eq!(TRUSTX_EIDAS2, "https://sparq.dev/ns/trust#eIDAS2");
        assert_eq!(TRUSTX_DIATF, "https://sparq.dev/ns/trust#DIATF");
        assert_eq!(SEC_REQ_EIDAS20, "https://w3id.org/zkp-sparql/sec-req#Eidas20");
        assert_eq!(SEC_REQ_UK_DVS, "https://w3id.org/zkp-sparql/sec-req#UkDvs");
    }

    #[test]
    fn all_trustx_iris_registry_is_complete() {
        // The registry must list every public constant above (21 total: 19 trustx: +
        // 2 sec-req: references). A guard against a new constant that the drift test
        // would then silently not cover.
        assert_eq!(
            ALL_TRUSTX_IRIS.len(),
            21,
            "ALL_TRUSTX_IRIS must list every minted constant + the 2 sec-req: references",
        );
    }
}
