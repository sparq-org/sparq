// AUTHORED-BY Claude Opus 4.8
//! Conformance/dev seeding of the in-memory store.
//!
//! The Solid Conformance Test Harness (CTH) drives the server entirely through HTTP, but it
//! **bootstraps** by dereferencing each test user's WebID (`pim:storage` → the pod root) and then
//! operating inside a `test/` container under that pod. With the in-memory store doubles nothing
//! exists at boot, so this module seeds the minimum the harness needs to begin:
//!
//! - the root container `/`,
//! - per-user `/{u}/`, `/{u}/profile/`, `/{u}/test/` containers,
//! - each user's WebID profile document `/{u}/profile/card` (the `#me` subject carries `pim:storage`
//!   → the pod root and `solid:oidcIssuer` → the trusted realm, which is what the harness reads),
//! - and the **Web Access Control ACLs** that make the pod owner-controlled: a pod-root ACL
//!   `/{u}/.acl` granting the owner Read/Write/Control on the root AND on all descendants
//!   (`acl:default`, so `/{u}/test/` etc. inherit owner control), plus a profile-card ACL granting
//!   the public `acl:Read` (so the WebID dereferences anonymously) and the owner full control.
//!
//! These ACLs are LOAD-BEARING once the WAC engine is enforced (this branch): without the pod-root
//! owner-default ACL, the owner could not create or manage ANY resource under their pod and the whole
//! conformance suite (Protocol + WAC) would fail-closed. They mirror prod-solid-server's provisioner
//! (`src/provisioning/provisioner.ts`).
//!
//! It is **dev/conformance only**, gated behind `SOLID_SERVER_SEED_CONFORMANCE=1` in `main`. It
//! never runs against a real (SPARQ/S3) backend in production.
//!
//! ## RDF construction
//! The WebID profile is built as `oxrdf::Triple`s and serialised with the server's own
//! [`serialize_triples`] (oxttl) — the house rule of never
//! hand-concatenating RDF. The container records are created through the public [`Store`] API
//! (`write` to mint the container's metadata record, `create_in_container` to wire containment), so
//! seeding exercises the same code path a real write would.

use axum::body::Bytes;
use oxrdf::{NamedNode, Triple};

use crate::error::ServerResult;
use crate::identity::{reserved_doc_iri, IdentityConfig};
use crate::ldp::content::{serialize_triples, RdfFormat};
use crate::store::Store;

/// The conformance test users. Each maps to a Keycloak service-account client whose token carries
/// the matching `webid` claim (`https://<base>/{u}/profile/card#me` in the default posture, or
/// `https://<identity-host>/{u}#me` in identity mode — see [`seed_conformance_with_identity`]).
pub const SEED_USERS: [&str; 2] = ["alice", "bob"];

/// Seed the store with the root container, the per-user container tree, and each user's WebID
/// profile. Idempotent-ish: intended to run once at boot on a fresh in-memory store.
///
/// `base_url` is the server's public origin without a trailing slash (e.g. `https://localhost:3000`).
/// `issuer` is the trusted token issuer recorded as each WebID's `solid:oidcIssuer`.
pub async fn seed_conformance<S: Store>(
    store: &S,
    base_url: &str,
    issuer: &str,
) -> ServerResult<()> {
    seed_conformance_with_identity(store, base_url, issuer, None).await
}

/// [`seed_conformance`] with the OPTIONAL identity-host posture (`research/lws-design-records.md`
/// §4).
///
/// - `identity: None` — byte-identical to the pre-identity seed: the WebID is the in-pod
///   `/{u}/profile/card#me` carrying `pim:storage` + `solid:oidcIssuer`.
/// - `identity: Some(config)` — **provider WebIDs are hosted OUTSIDE the pod**:
///   - each user's WebID becomes `https://<identity-host>/{u}#me`; its LOCKED id-doc (Person type,
///     provider-locked `solid:oidcIssuer`, `pim:storage` → the pod root, the
///     `<pod> solid:owner <webid>` back-link, `rdfs:seeAlso` → the in-pod card) is written at the
///     RESERVED store key `<base>/.identity/{u}` — via [`Store::write`], with NO containment edge,
///     so it appears in no `ldp:contains` listing and is addressable ONLY by the id-host route
///     (the LDP surface refuses the namespace outright);
///   - **no `.acl` is written for the id-doc** — none can exist (the namespace is refused), which
///     is the security property, not an omission;
///   - the pod-root ACL's owner `acl:agent` is the **id-host WebID** (the token's `webid` claim in
///     identity mode);
///   - the in-pod `/{u}/profile/card` is DEMOTED to a user-editable extended profile carrying
///     **no `solid:oidcIssuer` and no `pim:storage`** (nothing security-bearing may live in a
///     WAC-governed, owner-writable document).
pub async fn seed_conformance_with_identity<S: Store>(
    store: &S,
    base_url: &str,
    issuer: &str,
    identity: Option<&IdentityConfig>,
) -> ServerResult<()> {
    let base = base_url.trim_end_matches('/');

    // The root container must exist first (it is the parent of every per-user container, and the
    // harness GETs `/` to confirm the storage root).
    let root = format!("{base}/");
    ensure_container(store, &root, None).await?;

    for user in SEED_USERS {
        let pod = format!("{base}/{user}/");
        let profile = format!("{base}/{user}/profile/");
        let test = format!("{base}/{user}/test/");
        let card = format!("{base}/{user}/profile/card");
        // The OWNER WebID every ACL names: the id-host WebID in identity mode, else the in-pod card.
        let webid = match identity {
            Some(config) => config.webid(user),
            None => format!("{card}#me"),
        };

        // Container tree: /{u}/ ⊂ / ; /{u}/profile/ ⊂ /{u}/ ; /{u}/test/ ⊂ /{u}/.
        ensure_container(store, &pod, Some(&root)).await?;
        ensure_container(store, &profile, Some(&pod)).await?;
        ensure_container(store, &test, Some(&pod)).await?;

        // The in-pod profile document `/{u}/profile/card`, wired as a child of /{u}/profile/.
        // Identity mode DEMOTES it: no issuer, no storage — an extended profile that the locked
        // id-doc `rdfs:seeAlso`-points at, describing the SAME agent as the id-host WebID (never a
        // competing legacy person). `webid` here IS the id-host WebID (identity mode).
        let body = match identity {
            Some(_) => demoted_card_turtle(&card, &webid)?,
            None => webid_profile_turtle(&webid, &pod, issuer)?,
        };
        store
            .create_in_container(
                &profile,
                &card,
                Bytes::from(body),
                RdfFormat::Turtle.media_type(),
            )
            .await?;

        // Identity mode: the LOCKED id-doc, at the RESERVED key, with NO containment edge and NO
        // `.acl` (none can exist — the LDP surface refuses `/.identity/**` outright). Served
        // read-only by the id-host route; written only here (dev seed) and by the future admin
        // provisioning seam.
        if let Some(config) = identity {
            let id_doc_key = reserved_doc_iri(base, user);
            let id_doc_body = identity_doc_turtle(config, user, &pod, issuer, &card)?;
            store
                .write(
                    &id_doc_key,
                    Bytes::from(id_doc_body),
                    RdfFormat::Turtle.media_type(),
                )
                .await?;
        }

        // The pod-root ACL `/{u}/.acl`: owner Read/Write/Control on the pod root AND on all
        // descendants (`acl:default`), so the whole pod is owner-controlled unless a descendant ACL
        // overrides it. This is what lets the owner create + manage every test resource under
        // `/{u}/test/` once WAC is enforced. Stored as a plain `.acl` resource (its own bytes), via
        // `write` (it is an auxiliary resource, not a container child).
        let pod_acl = format!("{pod}.acl");
        let pod_acl_body = pod_root_acl_turtle(&pod, &webid)?;
        store
            .write(
                &pod_acl,
                Bytes::from(pod_acl_body),
                RdfFormat::Turtle.media_type(),
            )
            .await?;

        // The profile-card ACL `/{u}/profile/card.acl`: public `acl:Read` (so the WebID is
        // world-dereferenceable, which the harness + every Solid client need to bootstrap) plus owner
        // full control. Without this, an anonymous GET of the WebID card would be denied and the
        // harness could not discover `pim:storage`.
        let card_acl = format!("{card}.acl");
        let card_acl_body = profile_card_acl_turtle(&card, &webid)?;
        store
            .write(
                &card_acl,
                Bytes::from(card_acl_body),
                RdfFormat::Turtle.media_type(),
            )
            .await?;
    }

    Ok(())
}

/// Create a container's metadata record (so it `exists`) and wire it into `parent`'s containment.
///
/// A container is seeded as an empty `text/turtle` resource whose IRI ends in `/`; the LDP read path
/// renders its `ldp:contains` listing from the authoritative membership at GET time. When `parent`
/// is given, the container is recorded as the parent's child; the root (`parent: None`) is written
/// standalone.
async fn ensure_container<S: Store>(
    store: &S,
    iri: &str,
    parent: Option<&str>,
) -> ServerResult<()> {
    if store.exists(iri).await? {
        return Ok(());
    }
    match parent {
        // The root (or any parentless container): a plain write mints its record.
        None => {
            store
                .write(iri, Bytes::new(), RdfFormat::Turtle.media_type())
                .await?;
        }
        // A nested container: record it as a child of its parent (containment edge + record together).
        Some(p) => {
            store
                .create_in_container(p, iri, Bytes::new(), RdfFormat::Turtle.media_type())
                .await?;
        }
    }
    Ok(())
}

/// Build a minimal WebID profile document as Turtle, via `oxrdf` triples (never hand-concatenated).
///
/// The `#me` subject is typed `foaf:Person` + `solid:Account`-style and carries the two statements
/// the harness reads to bootstrap: `pim:storage` (→ the pod root) and `solid:oidcIssuer` (→ the
/// trusted realm). The card document itself (`foaf:PrimaryTopic`) points at `#me`.
fn webid_profile_turtle(webid: &str, pod_root: &str, issuer: &str) -> ServerResult<Vec<u8>> {
    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    const FOAF_PERSON: &str = "http://xmlns.com/foaf/0.1/Person";
    const FOAF_PRIMARY_TOPIC: &str = "http://xmlns.com/foaf/0.1/primaryTopic";
    const PIM_STORAGE: &str = "http://www.w3.org/ns/pim/space#storage";
    const SOLID_OIDC_ISSUER: &str = "http://www.w3.org/ns/solid/terms#oidcIssuer";

    // The card document subject (the resource URL, no fragment) and the `#me` agent subject.
    let card_doc = webid.split('#').next().unwrap_or(webid);

    // Helper: an owned NamedNode from a validated IRI string (these are all server-constructed, so
    // they are well-formed; map any (unreachable) error to a storage error rather than panic).
    let nn = |s: &str| -> ServerResult<NamedNode> {
        NamedNode::new(s)
            .map_err(|e| crate::error::ServerError::Storage(format!("invalid seed IRI {s}: {e}")))
    };

    let triples = vec![
        // <card> foaf:primaryTopic <#me> .
        Triple::new(nn(card_doc)?, nn(FOAF_PRIMARY_TOPIC)?, nn(webid)?),
        // <#me> a foaf:Person .
        Triple::new(nn(webid)?, nn(RDF_TYPE)?, nn(FOAF_PERSON)?),
        // <#me> pim:storage <pod_root> .
        Triple::new(nn(webid)?, nn(PIM_STORAGE)?, nn(pod_root)?),
        // <#me> solid:oidcIssuer <issuer> .
        Triple::new(nn(webid)?, nn(SOLID_OIDC_ISSUER)?, nn(issuer)?),
    ];

    serialize_triples(RdfFormat::Turtle, &triples)
}

/// Build the LOCKED identity document for `handle` — the provider-managed WebID doc served from the
/// id host (identity mode; `research/lws-design-records.md` §4). Subjects are on the IDENTITY
/// origin (the served IRIs), never the reserved store key. Carries exactly the provider-locked
/// statements: Person type, the locked `solid:oidcIssuer`, `pim:storage` → the pod root, the `<pod>
/// solid:owner <webid>` back-link, and `rdfs:seeAlso` → the demoted in-pod card. Built via `oxrdf`
/// triples (never hand-concatenated — the house rule).
fn identity_doc_turtle(
    config: &IdentityConfig,
    handle: &str,
    pod_root: &str,
    issuer: &str,
    card: &str,
) -> ServerResult<Vec<u8>> {
    const FOAF_PERSON: &str = "http://xmlns.com/foaf/0.1/Person";
    const FOAF_PRIMARY_TOPIC: &str = "http://xmlns.com/foaf/0.1/primaryTopic";
    const PIM_STORAGE: &str = "http://www.w3.org/ns/pim/space#storage";
    const SOLID_OIDC_ISSUER: &str = "http://www.w3.org/ns/solid/terms#oidcIssuer";
    const SOLID_OWNER: &str = "http://www.w3.org/ns/solid/terms#owner";
    const RDFS_SEE_ALSO: &str = "http://www.w3.org/2000/01/rdf-schema#seeAlso";

    let doc = config.doc_iri(handle);
    let webid = config.webid(handle);
    let nn = |s: &str| -> ServerResult<NamedNode> {
        NamedNode::new(s).map_err(|e| {
            crate::error::ServerError::Storage(format!("invalid identity seed IRI {s}: {e}"))
        })
    };

    let triples = vec![
        // <doc> foaf:primaryTopic <doc#me> .
        Triple::new(nn(&doc)?, nn(FOAF_PRIMARY_TOPIC)?, nn(&webid)?),
        // <doc#me> a foaf:Person .
        Triple::new(nn(&webid)?, nn(RDF_TYPE)?, nn(FOAF_PERSON)?),
        // <doc#me> pim:storage <pod_root> .   (locked — the demoted card carries none)
        Triple::new(nn(&webid)?, nn(PIM_STORAGE)?, nn(pod_root)?),
        // <doc#me> solid:oidcIssuer <issuer> .   (locked — the identity trust root)
        Triple::new(nn(&webid)?, nn(SOLID_OIDC_ISSUER)?, nn(issuer)?),
        // <pod_root> solid:owner <doc#me> .   (the per-user back-link, asserted in the PUBLIC
        // per-user id-doc — never aggregated into one enumerable server-wide document, the
        // ADR-0020 user-enumeration deviation)
        Triple::new(nn(pod_root)?, nn(SOLID_OWNER)?, nn(&webid)?),
        // <doc#me> rdfs:seeAlso <card> .   (the demoted, user-editable extended profile)
        Triple::new(nn(&webid)?, nn(RDFS_SEE_ALSO)?, nn(card)?),
    ];

    serialize_triples(RdfFormat::Turtle, &triples)
}

/// Build the DEMOTED in-pod profile card (identity mode): a user-editable extended profile carrying
/// **no `solid:oidcIssuer` and no `pim:storage`** — nothing security-bearing may live in a
/// WAC-governed, owner-writable document (the whole point of hosting the WebID outside the pod).
///
/// It is an HONEST EXTENSION of the id-host WebID, never a competing profile (roborev Finding 2):
/// - the card's `foaf:primaryTopic` is the **id-host WebID** — the document is ABOUT that agent, so
///   the id-doc's `rdfs:seeAlso → <card>` link resolves to a document extending the same person;
/// - the legacy `<card>#me` IRI is tied to the id-host WebID via `owl:sameAs` (and typed
///   `foaf:Person`), so a client that dereferences the legacy IRI learns it is the SAME agent — it
///   asserts no separate person.
fn demoted_card_turtle(card: &str, id_webid: &str) -> ServerResult<Vec<u8>> {
    const FOAF_PERSON: &str = "http://xmlns.com/foaf/0.1/Person";
    const FOAF_PRIMARY_TOPIC: &str = "http://xmlns.com/foaf/0.1/primaryTopic";
    const OWL_SAME_AS: &str = "http://www.w3.org/2002/07/owl#sameAs";

    let card_me = format!("{card}#me");
    let nn = |s: &str| -> ServerResult<NamedNode> {
        NamedNode::new(s).map_err(|e| {
            crate::error::ServerError::Storage(format!("invalid demoted-card IRI {s}: {e}"))
        })
    };
    let triples = vec![
        // <card> foaf:primaryTopic <id-webid> .   (the document extends the id-host WebID)
        Triple::new(nn(card)?, nn(FOAF_PRIMARY_TOPIC)?, nn(id_webid)?),
        // <card#me> owl:sameAs <id-webid> .   (the legacy IRI IS the id-host WebID's agent)
        Triple::new(nn(&card_me)?, nn(OWL_SAME_AS)?, nn(id_webid)?),
        // <card#me> a foaf:Person .   (still a person — just not a competing, security-bearing one)
        Triple::new(nn(&card_me)?, nn(RDF_TYPE)?, nn(FOAF_PERSON)?),
    ];
    serialize_triples(RdfFormat::Turtle, &triples)
}

// --- ACL vocabulary (built via oxrdf triples — never hand-concatenated, the house rule) -----------

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const ACL_AUTHORIZATION: &str = "http://www.w3.org/ns/auth/acl#Authorization";
const ACL_AGENT: &str = "http://www.w3.org/ns/auth/acl#agent";
const ACL_AGENT_CLASS: &str = "http://www.w3.org/ns/auth/acl#agentClass";
const ACL_ACCESS_TO: &str = "http://www.w3.org/ns/auth/acl#accessTo";
const ACL_DEFAULT: &str = "http://www.w3.org/ns/auth/acl#default";
const ACL_MODE: &str = "http://www.w3.org/ns/auth/acl#mode";
const ACL_READ: &str = "http://www.w3.org/ns/auth/acl#Read";
const ACL_WRITE: &str = "http://www.w3.org/ns/auth/acl#Write";
const ACL_APPEND: &str = "http://www.w3.org/ns/auth/acl#Append";
const ACL_CONTROL: &str = "http://www.w3.org/ns/auth/acl#Control";
const ACL_AUTHENTICATED_AGENT: &str = "http://www.w3.org/ns/auth/acl#AuthenticatedAgent";
const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";

/// A `NamedNode` from a server-constructed IRI (well-formed by construction; map an unexpected error
/// to a storage error rather than panic).
fn acl_nn(s: &str) -> ServerResult<NamedNode> {
    NamedNode::new(s)
        .map_err(|e| crate::error::ServerError::Storage(format!("invalid seed ACL IRI {s}: {e}")))
}

/// The pod-root ACL: the owner (`webid`) gets Read/Write/Control on the pod root (`acl:accessTo`) AND
/// on all descendants (`acl:default`), so the whole pod is owner-controlled unless a descendant ACL
/// overrides it. Authorization subject uses the conventional `<acl-doc>#owner` fragment.
fn pod_root_acl_turtle(pod_root: &str, webid: &str) -> ServerResult<Vec<u8>> {
    let acl_doc = format!("{pod_root}.acl");
    let auth = acl_nn(&format!("{acl_doc}#owner"))?;
    let root = acl_nn(pod_root)?;
    let me = acl_nn(webid)?;
    let triples = vec![
        Triple::new(auth.clone(), acl_nn(RDF_TYPE)?, acl_nn(ACL_AUTHORIZATION)?),
        Triple::new(auth.clone(), acl_nn(ACL_AGENT)?, me),
        Triple::new(auth.clone(), acl_nn(ACL_ACCESS_TO)?, root.clone()),
        Triple::new(auth.clone(), acl_nn(ACL_DEFAULT)?, root),
        Triple::new(auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
        Triple::new(auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_WRITE)?),
        Triple::new(auth, acl_nn(ACL_MODE)?, acl_nn(ACL_CONTROL)?),
    ];
    serialize_triples(RdfFormat::Turtle, &triples)
}

/// The profile-document ACL: the document is publicly readable (`acl:agentClass foaf:Agent` →
/// `acl:Read`) so the WebID dereferences for anyone; the owner additionally has Read/Write/Control.
fn profile_card_acl_turtle(profile_doc: &str, webid: &str) -> ServerResult<Vec<u8>> {
    let acl_doc = format!("{profile_doc}.acl");
    let owner_auth = acl_nn(&format!("{acl_doc}#owner"))?;
    let public_auth = acl_nn(&format!("{acl_doc}#public"))?;
    let doc = acl_nn(profile_doc)?;
    let me = acl_nn(webid)?;
    let triples = vec![
        // Owner: full control of the profile document.
        Triple::new(
            owner_auth.clone(),
            acl_nn(RDF_TYPE)?,
            acl_nn(ACL_AUTHORIZATION)?,
        ),
        Triple::new(owner_auth.clone(), acl_nn(ACL_AGENT)?, me),
        Triple::new(owner_auth.clone(), acl_nn(ACL_ACCESS_TO)?, doc.clone()),
        Triple::new(owner_auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
        Triple::new(owner_auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_WRITE)?),
        Triple::new(owner_auth, acl_nn(ACL_MODE)?, acl_nn(ACL_CONTROL)?),
        // Public: read-only (a WebID must be world-readable).
        Triple::new(
            public_auth.clone(),
            acl_nn(RDF_TYPE)?,
            acl_nn(ACL_AUTHORIZATION)?,
        ),
        Triple::new(
            public_auth.clone(),
            acl_nn(ACL_AGENT_CLASS)?,
            acl_nn(FOAF_AGENT)?,
        ),
        Triple::new(public_auth.clone(), acl_nn(ACL_ACCESS_TO)?, doc),
        Triple::new(public_auth, acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
    ];
    serialize_triples(RdfFormat::Turtle, &triples)
}

// --- Benchmark seeding (dev-only; gated by SOLID_SERVER_SEED_BENCH) --------------------------------
//
// Identical in nature to the conformance seed: it ONLY writes fixtures into the in-memory store at
// boot and changes NO request-handling code path. It exists so the HTTPS load benchmark
// (`bench/run.sh`) has stable, AUTH-FREE fixtures to measure the read hot path against without
// standing up Keycloak:
//   - a PUBLIC-readable RDF document,
//   - a PUBLIC-readable container with a configurable number of PUBLIC children (the listing path),
//   - a PRIVATE (owner-only) RDF document — present so an authenticated-throughput follow-up can
//     target it; it is NOT anonymously readable.
// The public fixtures live UNDER a dedicated `/{BENCH_USER}/` pod whose pod-root ACL grants
// `foaf:Agent acl:Read` by `acl:default`, so every descendant is anonymously readable for the read
// benchmark — and the private document carries its OWN owner-only `.acl` overriding that default.

/// The bench fixture pod owner (a synthetic WebID — bench fixtures are not tied to a real user).
pub const BENCH_USER: &str = "bench";
/// The default number of children seeded into the bench listing container (overridable via
/// `SOLID_SERVER_SEED_BENCH` = an integer; any non-integer / unset-but-flag-on ⇒ this default).
pub const BENCH_DEFAULT_CHILDREN: usize = 100;

/// Seed the deterministic benchmark fixtures into `store` (dev-only; see the module note above).
///
/// `base_url` is the server's public origin without a trailing slash. `child_count` is how many
/// public children to place in the listing container. Returns the seeded fixture IRIs (for the
/// bench harness / a log line) — the public doc, the listing container, and the private doc.
pub async fn seed_bench<S: Store>(
    store: &S,
    base_url: &str,
    child_count: usize,
) -> ServerResult<BenchFixtures> {
    seed_bench_with_owner(store, base_url, child_count, None).await
}

/// [`seed_bench`] with an explicit owner-WebID override for the ACL grants (dev-only, like the rest
/// of the seed).
///
/// WHY: the token verifier requires the `webid` claim to be an `https:` URL, but the DERIVED bench
/// owner (`<base>/bench/profile/card#me`) is an `http:` IRI whenever the server serves plain HTTP —
/// so under an http base no valid token could ever match the seeded owner-only ACL. The Linux
/// syscall harness (`bench/syscalls.sh`) measures over plain HTTP and passes a synthetic `https:`
/// owner WebID here (never dereferenced — it runs with `SOLID_SERVER_BIDIRECTIONAL=off`), minting
/// its tokens for the same WebID. `None` ⇒ the derived owner, byte-identical to before.
pub async fn seed_bench_with_owner<S: Store>(
    store: &S,
    base_url: &str,
    child_count: usize,
    owner_webid: Option<&str>,
) -> ServerResult<BenchFixtures> {
    let base = base_url.trim_end_matches('/');

    // Root + the bench pod must exist before anything under them.
    let root = format!("{base}/");
    ensure_container(store, &root, None).await?;
    let pod = format!("{base}/{BENCH_USER}/");
    ensure_container(store, &pod, Some(&root)).await?;

    // The bench WebID owner subject (used by the owner-only private-doc ACL).
    let owner = match owner_webid {
        Some(w) => w.to_string(),
        None => format!("{base}/{BENCH_USER}/profile/card#me"),
    };

    // The pod-root ACL: PUBLIC Read by default (`acl:default`, so descendants inherit it) PLUS the
    // owner full control. This is what makes the public doc + listing container anonymously readable.
    let pod_acl = format!("{pod}.acl");
    let pod_acl_body = public_read_default_acl_turtle(&pod, &owner)?;
    store
        .write(
            &pod_acl,
            Bytes::from(pod_acl_body),
            RdfFormat::Turtle.media_type(),
        )
        .await?;

    // (a) The PUBLIC document — a small RDF resource (inherits the pod-root public-read default).
    let public_doc = format!("{base}/{BENCH_USER}/public/doc");
    let public_dir = format!("{base}/{BENCH_USER}/public/");
    ensure_container(store, &public_dir, Some(&pod)).await?;
    let public_body = bench_doc_turtle(&public_doc, "public benchmark document")?;
    store
        .create_in_container(
            &public_dir,
            &public_doc,
            Bytes::from(public_body),
            RdfFormat::Turtle.media_type(),
        )
        .await?;

    // (b) The PUBLIC listing container with `child_count` children (inherits public-read default).
    let listing = format!("{base}/{BENCH_USER}/listing/");
    ensure_container(store, &listing, Some(&pod)).await?;
    for i in 0..child_count {
        let child = format!("{listing}item-{i:04}");
        let body = bench_doc_turtle(&child, &format!("listing child {i}"))?;
        store
            .create_in_container(
                &listing,
                &child,
                Bytes::from(body),
                RdfFormat::Turtle.media_type(),
            )
            .await?;
    }

    // (c) The PRIVATE document — owner-only. Its OWN `.acl` overrides the pod-root public default, so
    // an anonymous GET answers 401 (the auth-verify hot path the bench's authed follow-up targets).
    let private_dir = format!("{base}/{BENCH_USER}/private/");
    ensure_container(store, &private_dir, Some(&pod)).await?;
    let private_doc = format!("{base}/{BENCH_USER}/private/doc");
    let private_body = bench_doc_turtle(&private_doc, "private benchmark document")?;
    store
        .create_in_container(
            &private_dir,
            &private_doc,
            Bytes::from(private_body),
            RdfFormat::Turtle.media_type(),
        )
        .await?;
    // Owner-only ACL on the private document (no public grant) — overrides the inherited public read.
    let private_acl = format!("{private_doc}.acl");
    let private_acl_body = owner_only_acl_turtle(&private_doc, &owner)?;
    store
        .write(
            &private_acl,
            Bytes::from(private_acl_body),
            RdfFormat::Turtle.media_type(),
        )
        .await?;

    Ok(BenchFixtures {
        public_doc,
        listing,
        private_doc,
        child_count,
        owner,
    })
}

/// The IRIs the bench seed produced (echoed at boot so the harness/log shows exactly what to hit).
#[derive(Debug, Clone)]
pub struct BenchFixtures {
    pub public_doc: String,
    pub listing: String,
    pub private_doc: String,
    pub child_count: usize,
    /// The WebID the owner-only grants name (derived, or the dev-only override).
    pub owner: String,
}

/// A tiny RDF document body for a bench fixture: `<subject> rdfs:label "label"`. Built via oxrdf
/// triples + the server's own serialiser (never hand-concatenated — the house rule).
fn bench_doc_turtle(subject_iri: &str, label: &str) -> ServerResult<Vec<u8>> {
    const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
    let subject = NamedNode::new(subject_iri).map_err(|e| {
        crate::error::ServerError::Storage(format!("invalid bench IRI {subject_iri}: {e}"))
    })?;
    let pred = NamedNode::new(RDFS_LABEL)
        .map_err(|e| crate::error::ServerError::Storage(format!("invalid rdfs:label: {e}")))?;
    let triples = vec![Triple::new(
        subject,
        pred,
        oxrdf::Literal::new_simple_literal(label),
    )];
    serialize_triples(RdfFormat::Turtle, &triples)
}

/// A pod-root ACL granting the PUBLIC (`foaf:Agent`) `acl:Read` by default (inherited by descendants)
/// AND the owner full control. Used only for the bench fixtures' public-read posture.
fn public_read_default_acl_turtle(pod_root: &str, webid: &str) -> ServerResult<Vec<u8>> {
    let acl_doc = format!("{pod_root}.acl");
    let owner_auth = acl_nn(&format!("{acl_doc}#owner"))?;
    let public_auth = acl_nn(&format!("{acl_doc}#public"))?;
    let root = acl_nn(pod_root)?;
    let me = acl_nn(webid)?;
    let triples = vec![
        // Owner: full control on the root + descendants.
        Triple::new(
            owner_auth.clone(),
            acl_nn(RDF_TYPE)?,
            acl_nn(ACL_AUTHORIZATION)?,
        ),
        Triple::new(owner_auth.clone(), acl_nn(ACL_AGENT)?, me),
        Triple::new(owner_auth.clone(), acl_nn(ACL_ACCESS_TO)?, root.clone()),
        Triple::new(owner_auth.clone(), acl_nn(ACL_DEFAULT)?, root.clone()),
        Triple::new(owner_auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
        Triple::new(owner_auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_WRITE)?),
        Triple::new(owner_auth, acl_nn(ACL_MODE)?, acl_nn(ACL_CONTROL)?),
        // Public: Read on the root AND by default on descendants.
        Triple::new(
            public_auth.clone(),
            acl_nn(RDF_TYPE)?,
            acl_nn(ACL_AUTHORIZATION)?,
        ),
        Triple::new(
            public_auth.clone(),
            acl_nn(ACL_AGENT_CLASS)?,
            acl_nn(FOAF_AGENT)?,
        ),
        Triple::new(public_auth.clone(), acl_nn(ACL_ACCESS_TO)?, root.clone()),
        Triple::new(public_auth.clone(), acl_nn(ACL_DEFAULT)?, root),
        Triple::new(public_auth, acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
    ];
    serialize_triples(RdfFormat::Turtle, &triples)
}

/// An owner-only `.acl` on a single document (`acl:accessTo` only, no public grant, no `acl:default`)
/// — overrides an inherited public-read default so the document is owner-private.
fn owner_only_acl_turtle(doc: &str, webid: &str) -> ServerResult<Vec<u8>> {
    let acl_doc = format!("{doc}.acl");
    let owner_auth = acl_nn(&format!("{acl_doc}#owner"))?;
    let d = acl_nn(doc)?;
    let me = acl_nn(webid)?;
    let triples = vec![
        Triple::new(
            owner_auth.clone(),
            acl_nn(RDF_TYPE)?,
            acl_nn(ACL_AUTHORIZATION)?,
        ),
        Triple::new(owner_auth.clone(), acl_nn(ACL_AGENT)?, me),
        Triple::new(owner_auth.clone(), acl_nn(ACL_ACCESS_TO)?, d),
        Triple::new(owner_auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
        Triple::new(owner_auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_WRITE)?),
        Triple::new(owner_auth, acl_nn(ACL_MODE)?, acl_nn(ACL_CONTROL)?),
    ];
    serialize_triples(RdfFormat::Turtle, &triples)
}

// --- Demo playground seeding (demo-only; gated by SOLID_SERVER_SEED_DEMO) -------------------------
//
// The PUBLIC-demo posture of `research/lws-demo-architecture.md` §3.2. Identical in nature to the
// conformance/bench seeds: it ONLY writes fixtures into the in-memory store at boot and changes NO
// request-handling code path. Behind a public URL the write friction this demo relies on is
// REGISTRATION (any authenticated agent may write; anonymous visitors may not), so:
//   - a shared root-level `/playground/` container whose `.acl` grants any AUTHENTICATED agent
//     (`acl:agentClass acl:AuthenticatedAgent`) Read/Write/Append on the container AND
//     (`acl:default`) on everything created inside it, plus the public (`foaf:Agent`) Read the same
//     way — visitors with a (throwaway) WebID can write, anonymous visitors can only read. NO
//     `acl:Control` is granted to ANY principal: Control governs reading and rewriting the ACL
//     itself, so the sandbox can never be widened, locked, or hijacked over HTTP — the boot seed is
//     the ACL's only writer. Consequence, stated plainly: v1 visitors share ONE playground and are
//     NOT isolated from each other by WAC — only from anonymous writers.
//   - a PUBLIC-read `/README` Turtle document carrying the ephemeral-demo banner, so the demo's
//     properties are stated where every visitor can dereference them.

/// The `/README` label (`rdfs:label`) — what the demo instance calls itself.
pub const DEMO_README_LABEL: &str = "sparq LWS public demo";
/// The `/README` banner (`rdfs:comment`) — the §3.2 honesty print every visitor can dereference:
/// ephemeral, all data public-readable, wiped on idle, throwaway identities, no visitor isolation.
///
/// [OPUS-5] sq-5ougp review round 3 (gpt-5.6-sol finding 7): "not isolated from each other"
/// UNDER-DISCLOSED the consequence of the shared `acl:Write` grant. `acl:AuthenticatedAgent` +
/// `acl:Write` means any registered visitor may OVERWRITE and DELETE anyone else's resources, and
/// may publish arbitrary accepted RDF under the operator's origin. That posture is RATIFIED
/// (`research/lws-demo-architecture.md` §3.2, surfaced via proceed-and-document #2329 with the
/// steering window closed), so the fix is to DISCLOSE it plainly here — not to narrow the grant.
/// Pinned by `tests/demo_seed.rs::demo_seed_readme_banner_discloses_the_shared_write_consequences`.
pub const DEMO_README_BANNER: &str = "This is an EPHEMERAL public demo. Everything is wiped when \
    the instance idles out. All data is public-readable. Identities are throwaway. All visitors \
    share this one playground and are not isolated from each other: ANY registered visitor can \
    overwrite and delete anything you put here, and anything you publish is served from this \
    origin as-is. Do not store anything real, private, or of value here.";

/// The IRIs the demo seed produced (echoed at boot so the log shows exactly what to point a demo at).
#[derive(Debug, Clone)]
pub struct DemoFixtures {
    /// The shared sandbox container (authenticated read/write/append, public read, no Control).
    pub playground: String,
    /// The public-read banner document.
    pub readme: String,
}

/// Seed the §3.2 demo playground fixtures into `store` (demo-only; see the module note above).
///
/// `base_url` is the server's public origin without a trailing slash. Idempotent-ish, like the other
/// seeds: intended to run once at boot on a fresh in-memory store; a re-run does not error.
pub async fn seed_demo<S: Store>(store: &S, base_url: &str) -> ServerResult<DemoFixtures> {
    let base = base_url.trim_end_matches('/');

    // The root must exist before its children (nothing else in the root gets an ACL: everything
    // OUTSIDE the two seeded fixtures stays fail-closed).
    let root = format!("{base}/");
    ensure_container(store, &root, None).await?;

    // (a) The shared playground container + its no-Control ACL.
    let playground = format!("{base}/playground/");
    ensure_container(store, &playground, Some(&root)).await?;
    let playground_acl = format!("{playground}.acl");
    let playground_acl_body = demo_playground_acl_turtle(&playground)?;
    store
        .write(
            &playground_acl,
            Bytes::from(playground_acl_body),
            RdfFormat::Turtle.media_type(),
        )
        .await?;

    // (b) The public-read banner document + its read-only ACL.
    let readme = format!("{base}/README");
    let readme_body = demo_doc_turtle(&readme, DEMO_README_LABEL, DEMO_README_BANNER)?;
    store
        .create_in_container(
            &root,
            &readme,
            Bytes::from(readme_body),
            RdfFormat::Turtle.media_type(),
        )
        .await?;
    let readme_acl = format!("{readme}.acl");
    let readme_acl_body = public_read_acl_turtle(&readme)?;
    store
        .write(
            &readme_acl,
            Bytes::from(readme_acl_body),
            RdfFormat::Turtle.media_type(),
        )
        .await?;

    Ok(DemoFixtures { playground, readme })
}

/// A tiny demo RDF document: `<subject> rdfs:label "label" ; rdfs:comment "comment"`. Built via
/// oxrdf triples + the server's own serialiser (never hand-concatenated — the house rule).
fn demo_doc_turtle(subject_iri: &str, label: &str, comment: &str) -> ServerResult<Vec<u8>> {
    const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
    const RDFS_COMMENT: &str = "http://www.w3.org/2000/01/rdf-schema#comment";
    let nn = |s: &str| -> ServerResult<NamedNode> {
        NamedNode::new(s)
            .map_err(|e| crate::error::ServerError::Storage(format!("invalid demo IRI {s}: {e}")))
    };
    let subject = nn(subject_iri)?;
    let triples = vec![
        Triple::new(
            subject.clone(),
            nn(RDFS_LABEL)?,
            oxrdf::Literal::new_simple_literal(label),
        ),
        Triple::new(
            subject,
            nn(RDFS_COMMENT)?,
            oxrdf::Literal::new_simple_literal(comment),
        ),
    ];
    serialize_triples(RdfFormat::Turtle, &triples)
}

/// The §3.2 playground ACL: any AUTHENTICATED agent (`acl:agentClass acl:AuthenticatedAgent`) gets
/// `acl:Read` + `acl:Write` + `acl:Append` on the container (`acl:accessTo`) AND on everything
/// created inside it (`acl:default`); the public (`foaf:Agent`) gets `acl:Read` the same way.
/// Deliberately NO `acl:Control` for ANY principal — Control governs reading and rewriting the ACL
/// itself, so the sandbox can never be widened, locked, or hijacked over HTTP; the boot seed is the
/// ACL's only writer.
fn demo_playground_acl_turtle(container: &str) -> ServerResult<Vec<u8>> {
    let acl_doc = format!("{container}.acl");
    let authed_auth = acl_nn(&format!("{acl_doc}#authenticated"))?;
    let public_auth = acl_nn(&format!("{acl_doc}#public"))?;
    let c = acl_nn(container)?;
    let triples = vec![
        // Authenticated: Read/Write/Append on the container + everything created inside it.
        Triple::new(
            authed_auth.clone(),
            acl_nn(RDF_TYPE)?,
            acl_nn(ACL_AUTHORIZATION)?,
        ),
        Triple::new(
            authed_auth.clone(),
            acl_nn(ACL_AGENT_CLASS)?,
            acl_nn(ACL_AUTHENTICATED_AGENT)?,
        ),
        Triple::new(authed_auth.clone(), acl_nn(ACL_ACCESS_TO)?, c.clone()),
        Triple::new(authed_auth.clone(), acl_nn(ACL_DEFAULT)?, c.clone()),
        Triple::new(authed_auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
        Triple::new(authed_auth.clone(), acl_nn(ACL_MODE)?, acl_nn(ACL_WRITE)?),
        Triple::new(authed_auth, acl_nn(ACL_MODE)?, acl_nn(ACL_APPEND)?),
        // Public: Read-only, container + descendants (the all-data-public-readable banner claim).
        Triple::new(
            public_auth.clone(),
            acl_nn(RDF_TYPE)?,
            acl_nn(ACL_AUTHORIZATION)?,
        ),
        Triple::new(
            public_auth.clone(),
            acl_nn(ACL_AGENT_CLASS)?,
            acl_nn(FOAF_AGENT)?,
        ),
        Triple::new(public_auth.clone(), acl_nn(ACL_ACCESS_TO)?, c.clone()),
        Triple::new(public_auth.clone(), acl_nn(ACL_DEFAULT)?, c),
        Triple::new(public_auth, acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
    ];
    serialize_triples(RdfFormat::Turtle, &triples)
}

/// A PUBLIC-read-only `.acl` for a single document (`acl:agentClass foaf:Agent` → `acl:Read`,
/// `acl:accessTo` only): world-readable, writable by no one, `acl:Control` granted to no one.
fn public_read_acl_turtle(doc: &str) -> ServerResult<Vec<u8>> {
    let acl_doc = format!("{doc}.acl");
    let public_auth = acl_nn(&format!("{acl_doc}#public"))?;
    let d = acl_nn(doc)?;
    let triples = vec![
        Triple::new(
            public_auth.clone(),
            acl_nn(RDF_TYPE)?,
            acl_nn(ACL_AUTHORIZATION)?,
        ),
        Triple::new(
            public_auth.clone(),
            acl_nn(ACL_AGENT_CLASS)?,
            acl_nn(FOAF_AGENT)?,
        ),
        Triple::new(public_auth.clone(), acl_nn(ACL_ACCESS_TO)?, d),
        Triple::new(public_auth, acl_nn(ACL_MODE)?, acl_nn(ACL_READ)?),
    ];
    serialize_triples(RdfFormat::Turtle, &triples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};

    fn store() -> CompositeStore<InMemorySparqClient, InMemoryBlobStore> {
        CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new())
    }

    #[tokio::test]
    async fn seeds_root_users_and_webids() {
        let s = store();
        let base = "https://localhost:3000";
        let issuer = "http://localhost:8080/realms/solid";
        seed_conformance(&s, base, issuer).await.unwrap();

        // Root + per-user containers exist.
        assert!(s.exists("https://localhost:3000/").await.unwrap());
        for u in SEED_USERS {
            assert!(s
                .exists(&format!("https://localhost:3000/{u}/"))
                .await
                .unwrap());
            assert!(s
                .exists(&format!("https://localhost:3000/{u}/profile/"))
                .await
                .unwrap());
            assert!(s
                .exists(&format!("https://localhost:3000/{u}/test/"))
                .await
                .unwrap());
            assert!(s
                .exists(&format!("https://localhost:3000/{u}/profile/card"))
                .await
                .unwrap());
        }

        // The WebID card carries pim:storage + solid:oidcIssuer.
        let card = s
            .read("https://localhost:3000/alice/profile/card")
            .await
            .unwrap();
        let body = String::from_utf8(card.body.to_vec()).unwrap();
        assert!(body.contains("pim/space#storage"));
        assert!(body.contains("solid/terms#oidcIssuer"));
        assert!(body.contains("https://localhost:3000/alice/"));
        assert!(body.contains(issuer));
    }

    #[tokio::test]
    async fn seeds_owner_acls_and_wac_grants_owner_full_control() {
        use crate::authz::wac::{Decision, WacAuthorizer};
        use crate::authz::AccessMode;

        let s = store();
        let base = "https://localhost:3000";
        let issuer = "http://localhost:8080/realms/solid";
        seed_conformance(&s, base, issuer).await.unwrap();

        // The pod-root + profile-card ACLs exist.
        assert!(s.exists("https://localhost:3000/alice/.acl").await.unwrap());
        assert!(s
            .exists("https://localhost:3000/alice/profile/card.acl")
            .await
            .unwrap());

        let alice = "https://localhost:3000/alice/profile/card#me";
        let bob = "https://localhost:3000/bob/profile/card#me";
        let wac = WacAuthorizer::new(&s, base);

        // Alice (owner) inherits Read/Write/Control over a resource she'd create under /alice/test/
        // (via the pod-root `acl:default`).
        let target = "https://localhost:3000/alice/test/data";
        assert!(matches!(
            wac.authorize(target, AccessMode::Write, Some(alice), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        // Bob is NOT granted on Alice's pod → 403.
        assert_eq!(
            wac.authorize(target, AccessMode::Read, Some(bob), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );

        // The WebID profile card is PUBLIC-readable (anonymous GET allowed) but NOT public-writable.
        let card = "https://localhost:3000/alice/profile/card";
        assert!(matches!(
            wac.authorize(card, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        assert_eq!(
            wac.authorize(card, AccessMode::Write, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        // Alice fully controls her own card.
        assert!(matches!(
            wac.authorize(card, AccessMode::Control, Some(alice), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    #[tokio::test]
    async fn webid_profile_is_valid_turtle() {
        let body = webid_profile_turtle(
            "https://localhost:3000/alice/profile/card#me",
            "https://localhost:3000/alice/",
            "http://localhost:8080/realms/solid",
        )
        .unwrap();
        // Re-parse to confirm well-formed Turtle (round-trips through oxttl).
        let n = crate::ldp::content::validate_rdf(
            RdfFormat::Turtle,
            &body,
            "https://localhost:3000/alice/profile/card",
        )
        .unwrap();
        assert_eq!(n, 4, "four triples in the seeded profile");
    }

    #[tokio::test]
    async fn seeds_bench_fixtures_public_and_private() {
        use crate::authz::wac::{Decision, WacAuthorizer};
        use crate::authz::AccessMode;

        let s = store();
        let base = "https://localhost:3000";
        let fx = seed_bench(&s, base, 10).await.unwrap();

        // The public doc + listing container exist, and the listing has exactly the seeded children.
        assert!(s.exists(&fx.public_doc).await.unwrap());
        assert!(s.exists(&fx.listing).await.unwrap());
        assert!(s.exists(&fx.private_doc).await.unwrap());
        assert_eq!(fx.child_count, 10);
        assert_eq!(s.list_children(&fx.listing).await.unwrap().len(), 10);

        let wac = WacAuthorizer::new(&s, base);
        // The public doc + listing container are ANONYMOUSLY readable (public-read default).
        assert!(matches!(
            wac.authorize(&fx.public_doc, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        assert!(matches!(
            wac.authorize(&fx.listing, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        // The private doc is NOT anonymously readable (its own owner-only ACL overrides the default).
        assert_eq!(
            wac.authorize(&fx.private_doc, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        // The owner CAN read the private doc.
        let owner = format!("{base}/{BENCH_USER}/profile/card#me");
        assert!(matches!(
            wac.authorize(&fx.private_doc, AccessMode::Read, Some(&owner), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    #[tokio::test]
    async fn bench_owner_override_grants_exactly_that_webid() {
        use crate::authz::wac::{Decision, WacAuthorizer};
        use crate::authz::AccessMode;

        let s = store();
        // Plain-HTTP base (the syscall-harness posture): the DERIVED owner would be an http: IRI no
        // valid https-WebID token could match — the override names the https WebID instead.
        let base = "http://127.0.0.1:3400";
        let owner = "https://bench.example/profile/card#me";
        let fx = seed_bench_with_owner(&s, base, 3, Some(owner))
            .await
            .unwrap();
        assert_eq!(fx.owner, owner);

        let wac = WacAuthorizer::new(&s, base);
        // The override owner reads the private doc; the derived http owner does NOT.
        assert!(matches!(
            wac.authorize(&fx.private_doc, AccessMode::Read, Some(owner), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        let derived = format!("{base}/{BENCH_USER}/profile/card#me");
        assert!(!matches!(
            wac.authorize(&fx.private_doc, AccessMode::Read, Some(&derived), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        // The override owner also gets Write under the pod default (the harness PUT scenario).
        let put_target = format!("{base}/bench/private/put-target");
        assert!(matches!(
            wac.authorize(&put_target, AccessMode::Write, Some(owner), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        // And the public fixtures stay anonymously readable.
        assert!(matches!(
            wac.authorize(&fx.public_doc, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
    }

    #[tokio::test]
    async fn seeds_demo_playground_authenticated_write_public_read() {
        use crate::authz::wac::{Decision, WacAuthorizer};
        use crate::authz::AccessMode;

        let s = store();
        let base = "https://localhost:3000";
        let fx = seed_demo(&s, base).await.unwrap();

        // The playground container and the README both exist, at the §3.2 root-level IRIs.
        assert_eq!(fx.playground, format!("{base}/playground/"));
        assert_eq!(fx.readme, format!("{base}/README"));
        assert!(s.exists(&fx.playground).await.unwrap());
        assert!(s.exists(&fx.readme).await.unwrap());

        // The README carries the ephemeral-demo banner and is ANONYMOUSLY readable but writable by
        // NO ONE (not even an authenticated visitor).
        let readme = s.read(&fx.readme).await.unwrap();
        let body = String::from_utf8(readme.body.to_vec()).unwrap();
        assert!(body.contains("EPHEMERAL"), "banner text missing: {body}");
        let wac = WacAuthorizer::new(&s, base);
        let visitor = "https://css-idp.example/visitor/profile/card#me";
        assert!(matches!(
            wac.authorize(&fx.readme, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        assert_eq!(
            wac.authorize(&fx.readme, AccessMode::Write, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        assert_eq!(
            wac.authorize(&fx.readme, AccessMode::Write, Some(visitor), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );

        // Inside the playground, ANY authenticated agent gets Read/Write/Append — including on a
        // not-yet-existing child (creation flows through the container's `acl:default`) — while
        // ANONYMOUS visitors get Read only.
        let scratch = format!("{}scratch", fx.playground);
        for mode in [AccessMode::Read, AccessMode::Write, AccessMode::Append] {
            assert!(matches!(
                wac.authorize(&fx.playground, mode, Some(visitor), None)
                    .await
                    .unwrap(),
                Decision::Allow(_)
            ));
            assert!(matches!(
                wac.authorize(&scratch, mode, Some(visitor), None)
                    .await
                    .unwrap(),
                Decision::Allow(_)
            ));
        }
        assert!(matches!(
            wac.authorize(&fx.playground, AccessMode::Read, None, None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        assert_eq!(
            wac.authorize(&fx.playground, AccessMode::Write, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        assert_eq!(
            wac.authorize(&scratch, AccessMode::Write, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );

        // NOBODY has Control — the ACL cannot be widened, locked, or hijacked over HTTP.
        assert_eq!(
            wac.authorize(&fx.playground, AccessMode::Control, Some(visitor), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
        assert_eq!(
            wac.authorize(&fx.playground, AccessMode::Control, None, None)
                .await
                .unwrap(),
            Decision::Unauthenticated
        );
        // Belt-and-braces: the serialized playground ACL carries NO acl:Control grant at all.
        let acl = s
            .read(&format!("{}.acl", fx.playground))
            .await
            .unwrap();
        let acl_body = String::from_utf8(acl.body.to_vec()).unwrap();
        assert!(
            !acl_body.contains("acl#Control"),
            "the playground ACL must grant no Control: {acl_body}"
        );
        assert!(acl_body.contains("acl#AuthenticatedAgent"));
    }

    #[tokio::test]
    async fn demo_seed_is_idempotent() {
        let s = store();
        let base = "https://localhost:3000";
        let fx = seed_demo(&s, base).await.unwrap();
        // A second run must not error (already-exists short-circuits; ACL/doc writes overwrite).
        let again = seed_demo(&s, base).await.unwrap();
        assert_eq!(fx.playground, again.playground);
        assert_eq!(fx.readme, again.readme);
        assert!(s.exists(&fx.playground).await.unwrap());
        assert!(s.exists(&fx.readme).await.unwrap());
    }

    #[tokio::test]
    async fn seeding_is_idempotent() {
        let s = store();
        let base = "https://localhost:3000";
        let issuer = "http://localhost:8080/realms/solid";
        seed_conformance(&s, base, issuer).await.unwrap();
        // A second run must not error (already-exists short-circuits).
        seed_conformance(&s, base, issuer).await.unwrap();
        assert!(s
            .exists("https://localhost:3000/alice/test/")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn identity_seed_writes_locked_id_docs_outside_the_pod() {
        let s = store();
        let base = "https://localhost:3000";
        let issuer = "http://localhost:8080/realms/solid";
        let config = IdentityConfig::new(base, None).unwrap();
        seed_conformance_with_identity(&s, base, issuer, Some(&config))
            .await
            .unwrap();

        // The id-doc lives at the RESERVED key (outside the LDP-addressable surface).
        let key = "https://localhost:3000/.identity/alice";
        assert!(s.exists(key).await.unwrap());
        let doc = s.read(key).await.unwrap();
        let body = String::from_utf8(doc.body.to_vec()).unwrap();
        // Subjects are on the IDENTITY origin, and the locked statements are all present.
        assert!(body.contains("id.localhost:3000/alice#me"));
        assert!(body.contains("pim/space#storage"));
        assert!(body.contains("solid/terms#oidcIssuer"));
        assert!(body.contains("solid/terms#owner"));
        assert!(body.contains(issuer));
        assert!(body.contains("https://localhost:3000/alice/")); // pim:storage + solid:owner subject
        assert!(body.contains("rdf-schema#seeAlso"));

        // NO `.acl` exists for the id-doc — none can (the namespace is refused on the LDP surface).
        assert!(!s
            .exists("https://localhost:3000/.identity/alice.acl")
            .await
            .unwrap());

        // The id-doc has NO containment edge: the root listing exposes only the user pods, never
        // the reserved namespace (outside the LDP-resource→storage mapping).
        let root_children = s.list_children("https://localhost:3000/").await.unwrap();
        assert!(
            root_children
                .iter()
                .all(|c| !c.as_str().contains(".identity")),
            "the reserved namespace must never appear in an ldp:contains listing: {root_children:?}"
        );
    }

    #[tokio::test]
    async fn identity_seed_demotes_the_in_pod_card() {
        let s = store();
        let base = "https://localhost:3000";
        let issuer = "http://localhost:8080/realms/solid";
        let config = IdentityConfig::new(base, None).unwrap();
        seed_conformance_with_identity(&s, base, issuer, Some(&config))
            .await
            .unwrap();

        // The demoted card exists but carries NOTHING security-bearing: no issuer, no storage.
        let card = s
            .read("https://localhost:3000/alice/profile/card")
            .await
            .unwrap();
        let body = String::from_utf8(card.body.to_vec()).unwrap();
        assert!(
            !body.contains("oidcIssuer"),
            "the demoted card must not carry solid:oidcIssuer: {body}"
        );
        assert!(
            !body.contains("pim/space#storage"),
            "the demoted card must not carry pim:storage: {body}"
        );
        // HONEST EXTENSION, not a competing profile (roborev Finding 2): the card's primaryTopic is
        // the ID-HOST WebID, and the legacy `card#me` is owl:sameAs that WebID — so a client that
        // dereferences the legacy IRI learns it is the SAME agent, never a separate person.
        let id_webid = "https://id.localhost:3000/alice#me";
        assert!(body.contains("primaryTopic"));
        assert!(
            body.contains("2002/07/owl#sameAs"),
            "the demoted card must tie the legacy IRI to the id-host WebID via owl:sameAs: {body}"
        );
        assert!(
            body.contains(id_webid),
            "the demoted card's primaryTopic + owl:sameAs must name the id-host WebID: {body}"
        );
    }

    #[tokio::test]
    async fn identity_seed_binds_the_pod_acl_to_the_id_host_webid() {
        use crate::authz::wac::{Decision, WacAuthorizer};
        use crate::authz::AccessMode;

        let s = store();
        let base = "https://localhost:3000";
        let issuer = "http://localhost:8080/realms/solid";
        let config = IdentityConfig::new(base, None).unwrap();
        seed_conformance_with_identity(&s, base, issuer, Some(&config))
            .await
            .unwrap();

        let wac = WacAuthorizer::new(&s, base);
        let id_webid = config.webid("alice"); // https://id.localhost:3000/alice#me
        let old_webid = "https://localhost:3000/alice/profile/card#me";
        let target = "https://localhost:3000/alice/test/data";

        // The ID-HOST WebID is the pod owner (Read/Write/Control via the pod-root acl:default)…
        assert!(matches!(
            wac.authorize(target, AccessMode::Write, Some(&id_webid), None)
                .await
                .unwrap(),
            Decision::Allow(_)
        ));
        // …and the OLD in-pod WebID form is NOT granted (the ACL names only the id-host WebID).
        assert_eq!(
            wac.authorize(target, AccessMode::Write, Some(old_webid), None)
                .await
                .unwrap(),
            Decision::Forbidden
        );
    }
}
