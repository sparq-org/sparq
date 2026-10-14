//! The session layer: read the materialized auth view (`<urn:sparq:auth>`) into a
//! transient index, expand a (WebID, client-id) session into its principal set, and
//! compute the accessible graph set per mode — `∪ allow ∖ ∪ deny`, with conditional
//! grants (ACP `noneOf`) gated by their exception matchers.
//!
//! Everything here is derivable from the auth-view TRIPLES with plain SPARQL (the
//! design doc shows the MINUS form); this index is the cached fast path (D3).

use crate::loader::graph_triples;
use crate::{AUTH_GRAPH, AUTH_NS};
use oxrdf::{NamedNode, Term};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::Graph;

/// Principal IRI matched by every session (WAC `foaf:Agent` / ACP `acp:PublicAgent`).
pub const PUBLIC: &str = "https://sparq.dev/ns/auth#Public";
/// Principal IRI matched by any session with a WebID (WAC `acl:AuthenticatedAgent` /
/// ACP `acp:AuthenticatedAgent`).
pub const AUTHENTICATED: &str = "https://sparq.dev/ns/auth#Authenticated";
/// Client IRI matched by any client in conditional-grant heads (ACP `acp:PublicClient`).
pub const ANY_CLIENT: &str = "https://sparq.dev/ns/auth#AnyClient";
/// Issuer IRI matched by any OIDC issuer (the issuer-dimension top; ACP has no "public
/// issuer" special, so an absent `acp:issuer` is issuer-unconstrained ⇒ this top).
/// [OPUS-4.8] sq-3jtd.6.
pub const ANY_ISSUER: &str = "https://sparq.dev/ns/auth#AnyIssuer";

/// A request context: who (WebID) through what (client identifier / origin), vouched
/// for by which OIDC issuer (`acp:issuer`), and — for [OPUS-4.8] sq-0q7n — *when* (the
/// wall-clock instant the request is evaluated at). `agent: None` = anonymous;
/// `now: None` = no clock supplied.
///
/// A session expands to at most 12 principals when grants are looked up ([OPUS-4.8]
/// sq-3jtd.6 — the issuer dimension doubles the pre-issuer ≤6): always `auth:Public`;
/// plus `auth:Authenticated` and the WebID itself when `agent` is given; for each of
/// those agent-principals `ap`, the minted `urn:sparq:pair?agent=…&client=…` pair when
/// `client` is given, the minted `urn:sparq:triple?agent=…&client=…&issuer=…` term when
/// `issuer` is given (with the client component being `auth:AnyClient` when no `client`),
/// and the full triple when both are given. Expansion is internal — callers just supply
/// the request context:
///
/// # The clock (`now`) — [OPUS-4.8] sq-0q7n
///
/// `now` is the request instant as an `xsd:dateTime` lexical string (e.g.
/// `"2026-06-17T09:00:00Z"`). It is consulted ONLY by time-windowed conditional grants
/// (an `odrl:dateTime` constraint persisted as `auth:notBefore`/`auth:notAfter`
/// bounds): the grant applies iff `now` falls inside the window, re-checked *per
/// request* rather than frozen at materialization. **Fail-closed:** a time-windowed
/// grant evaluated with `now == None` (no clock evidence) never applies — exactly
/// mirroring the ODRL evaluator, which fails a `dateTime` constraint with no
/// request-context value. A grant with NO time window ignores `now`, so an
/// identity-only session need not supply it. Construct it with [`Session::at`].
///
/// # Examples
///
/// ```
/// use sparq_solid::Session;
///
/// let anonymous = Session::default();
/// let alice =
///     Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
/// let alice_via_app = Session {
///     agent: Some("https://alice.ex/card#me"),
///     client: Some("https://app.ex"),
///     issuer: Some("https://idp.ex"),
///     now: None,
/// };
/// // bind the request clock for a time-windowed grant:
/// let alice_at = alice.at("2026-06-17T09:00:00Z");
/// assert!(anonymous.agent.is_none());
/// assert_eq!(alice_at.now, Some("2026-06-17T09:00:00Z"));
/// # let _ = alice_via_app;
/// ```
///
/// # Invariants (fail-closed)
///
/// Agent/client/issuer values inside the reserved `urn:sparq:` IRI space, or containing
/// the literal pair-encoding delimiter `&client=`, are never matched against grants: the
/// accessible set for such a session is empty (they could otherwise impersonate a
/// minted pair/triple principal — see [`AuthIndex::accessible`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct Session<'a> {
    /// The authenticated agent's WebID; `None` = anonymous.
    pub agent: Option<&'a str>,
    /// The client identifier (WAC `acl:origin` / ACP `acp:client`); `None` = any client.
    pub client: Option<&'a str>,
    /// The OIDC issuer that vouched for the WebID (ACP `acp:issuer`); `None` = any issuer.
    /// [OPUS-4.8] sq-3jtd.6 — only ACP matchers constrain on it; WAC has no issuer notion,
    /// so a WAC pod ignores it (no grant ever names a triple principal).
    pub issuer: Option<&'a str>,
    /// The request instant as an `xsd:dateTime` lexical string, consulted only by
    /// time-windowed conditional grants ([OPUS-4.8] sq-0q7n). `None` = no clock supplied
    /// → a time-windowed grant fails closed.
    pub now: Option<&'a str>,
}

impl<'a> Session<'a> {
    /// This session with its request clock bound to `now` (an `xsd:dateTime` lexical
    /// string). Use it so a time-windowed conditional grant re-checks the live clock per
    /// request instead of being frozen at materialization. [OPUS-4.8] sq-0q7n.
    ///
    /// ```
    /// use sparq_solid::Session;
    /// let alice =
    ///     Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
    /// assert_eq!(alice.at("2026-06-17T00:00:00Z").now, Some("2026-06-17T00:00:00Z"));
    /// ```
    pub fn at(self, now: &'a str) -> Session<'a> {
        Session { now: Some(now), ..self }
    }
}

/// An access mode, matching WAC's `acl:Read`/`Write`/`Append`/`Control` (ACP reuses
/// the same four mode IRIs by convention).
///
/// Per the WAC spec, `Control` governs the access-control document itself: the rules
/// translate `acl:Control` on a resource into `auth:read`/`auth:write` grants **on
/// its `.acl` graph**, not into access to the resource.
///
/// # Examples
///
/// ```
/// use sparq_solid::Mode;
/// // Modes key the per-session cache; they are plain copyable enum values.
/// let modes = [Mode::Read, Mode::Write, Mode::Append, Mode::Control];
/// assert_eq!(modes.len(), 4);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// `acl:Read` — read the resource's triples.
    Read,
    /// `acl:Write` — replace/delete the resource's triples.
    Write,
    /// `acl:Append` — add triples without removing existing ones.
    Append,
    /// `acl:Control` — read/write the resource's access-control document.
    Control,
}

impl Mode {
    fn from_pred(p: &str) -> Option<(Mode, bool)> {
        let local = p.strip_prefix(AUTH_NS)?;
        Some(match local {
            "read" => (Mode::Read, true),
            "write" => (Mode::Write, true),
            "append" => (Mode::Append, true),
            "control" => (Mode::Control, true),
            "denyRead" => (Mode::Read, false),
            "denyWrite" => (Mode::Write, false),
            "denyAppend" => (Mode::Append, false),
            "denyControl" => (Mode::Control, false),
            _ => return None,
        })
    }

    fn from_mode_iri(iri: &str) -> Option<Mode> {
        Some(match iri {
            "http://www.w3.org/ns/auth/acl#Read" => Mode::Read,
            "http://www.w3.org/ns/auth/acl#Write" => Mode::Write,
            "http://www.w3.org/ns/auth/acl#Append" => Mode::Append,
            "http://www.w3.org/ns/auth/acl#Control" => Mode::Control,
            _ => return None,
        })
    }
}

/// The deterministic pair-principal IRI minted by the rules
/// (`string:encodeForUri` + `string:concatenation` in rules/wac.n3 / rules/acp-c.n3 —
/// keep in sync; both sides share [`sparq_reason::n3::encode_for_uri`]'s RFC 3986
/// percent-encoding, so the minting is INJECTIVE — no agent/client value can smuggle
/// the `&client=` delimiter into someone else's pair. The loader's reserved-encoding
/// validation ([`Session`] values, ACL agent/origin IRIs) stays as defense in depth.)
///
/// ```
/// use sparq_solid::pair_principal;
/// assert_eq!(
///     pair_principal("https://bob.ex/card#me", "https://app.ex"),
///     "urn:sparq:pair?agent=https%3A%2F%2Fbob.ex%2Fcard%23me&client=https%3A%2F%2Fapp.ex",
/// );
/// // injective: a delimiter inside a value cannot re-bracket the pair
/// assert_ne!(pair_principal("a&client=b", "c"), pair_principal("a", "b&client=c"));
/// ```
pub fn pair_principal(agent: &str, client: &str) -> String {
    let (a, c) = (sparq_reason::n3::encode_for_uri(agent), sparq_reason::n3::encode_for_uri(client));
    format!("urn:sparq:pair?agent={a}&client={c}")
}

/// [OPUS-4.8] sq-3jtd.6 — the deterministic THREE-dimension principal IRI minted by the
/// ACP rules for an issuer-constrained grant (`string:encodeForUri` +
/// `string:concatenation` in rules/acp-c.n3 — keep in sync; the same RFC 3986
/// percent-encoding makes the minting INJECTIVE, so no agent/client/issuer value can
/// smuggle a `&client=`/`&issuer=` delimiter into another principal's term).
///
/// The `client` component is `auth:AnyClient` (the crate-internal `ANY_CLIENT`) for a
/// matcher that constrains the issuer but not the client; only an issuer-CONSTRAINED
/// candidate mints a triple — an issuer-unconstrained grant reuses the agent /
/// [`pair_principal`] term.
///
/// ```
/// use sparq_solid::triple_principal;
/// assert_eq!(
///     triple_principal("https://bob.ex/card#me", "https://app.ex", "https://idp.ex"),
///     "urn:sparq:triple?agent=https%3A%2F%2Fbob.ex%2Fcard%23me\
///      &client=https%3A%2F%2Fapp.ex&issuer=https%3A%2F%2Fidp.ex",
/// );
/// // injective: a delimiter inside a value cannot re-bracket the triple
/// assert_ne!(
///     triple_principal("a&issuer=b", "c", "d"),
///     triple_principal("a", "c", "b&issuer=d"),
/// );
/// ```
pub fn triple_principal(agent: &str, client: &str, issuer: &str) -> String {
    let a = sparq_reason::n3::encode_for_uri(agent);
    let c = sparq_reason::n3::encode_for_uri(client);
    let i = sparq_reason::n3::encode_for_uri(issuer);
    format!("urn:sparq:triple?agent={a}&client={c}&issuer={i}")
}

#[derive(Debug, Default)]
struct ConditionalGrant {
    allow: bool,
    agent: String,  // principal-space: WebID | auth:Public | auth:Authenticated
    client: String, // auth:AnyClient | concrete client id
    // [OPUS-4.8] sq-3jtd.6: auth:AnyIssuer (issuer-unconstrained) | concrete OIDC issuer id.
    issuer: String,
    mode: Option<Mode>,
    graph: Option<NamedNode>,
    except: Vec<String>, // matcher IRIs
    // [OPUS-4.8] sq-0q7n: an optional live-clock window from a faithfully-mappable
    // `odrl:dateTime` constraint. `not_before` = the grant is inactive UNTIL this
    // instant (inclusive); `not_after` = inactive AFTER it (inclusive). Both are
    // `xsd:dateTime` lexical strings, compared against `Session::now` per request (see
    // [`AuthIndex::cond_applies`]). `None` on a bound means that side is unbounded.
    not_before: Option<String>,
    not_after: Option<String>,
}

/// A transient index over the `<urn:sparq:auth>` graph's triples: principal × mode →
/// graph names, plus conditional grants (ACP `noneOf`) and the matcher accept-sets
/// they reference.
///
/// Everything the index answers is derivable from the auth-view **triples** with
/// plain SPARQL (the design doc shows the `MINUS` form); the index is the cached fast
/// path (design doc D3) and holds no state of its own — [`crate::PodStore`] rebuilds
/// it from the triples after every re-materialization. Use it directly only for
/// inspection/tests ([`crate::PodStore::auth`]) or if you manage materialization
/// yourself with the free [`crate::materialize_wac`] / [`crate::materialize_acp`]
/// functions.
///
/// # Examples
///
/// ```
/// use sparq_solid::{materialize_wac, AuthIndex, Mode, Session};
///
/// let nquads = r#"
/// <https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
/// <https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
/// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
/// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
/// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
/// "#;
/// let mut graph = sparq_core::Graph::load_dataset(nquads, "nquads")?;
/// materialize_wac(&mut graph)?;
///
/// let index = AuthIndex::from_graph(&graph);
/// let alice = Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
/// assert_eq!(index.accessible(&alice, Mode::Read).len(), 2); // n1 + notes/
/// assert!(index.accessible(&alice, Mode::Write).is_empty()); // fail-closed
/// # Ok::<(), String>(())
/// ```
#[derive(Debug, Default)]
pub struct AuthIndex {
    /// (principal IRI, mode) → per-graph-ORIGIN → graphs, for simple allow / deny triples.
    ///
    /// [OPUS-4.8] sq-b7k7u: the graph set of each (principal, mode) is bucketed by the
    /// GRAPH's origin (`scheme://authority`, [`crate::loader::iri_origin`]). The flat set
    /// the audited walk used before is exactly the union of these buckets, so
    /// [`AuthIndex::accessible`] is byte-for-byte unchanged; the buckets additionally let
    /// [`AuthIndex::accessible_in_origin`] answer a decision restricted to one origin
    /// without touching the others — the substrate for `PodStore`'s per-origin decide and
    /// its **diff-based** session-cache invalidation on an ACL write (issue #1571,
    /// sq-b7k7u fix). Sound by construction: `reindex_with` diffs old vs new `AuthIndex`
    /// per-origin and invalidates exactly the origins whose buckets changed — no
    /// confinement lemma assumed (WAC agentGroup membership, foreign-subject triples, and
    /// ACP cross-document indirection can all let a write at origin A affect origin B's
    /// grants; the diff catches every such case automatically).
    allow: FxHashMap<(String, Mode), FxHashMap<String, Vec<NamedNode>>>,
    deny: FxHashMap<(String, Mode), FxHashMap<String, Vec<NamedNode>>>,
    /// Conditional (ACP `noneOf`) grants, bucketed by their target graph's origin. A grant
    /// with no target graph (inert — it inserts nothing) is bucketed under the empty-origin
    /// key; it is still visited by the all-origins walk, so the union stays exact.
    cond: FxHashMap<String, Vec<ConditionalGrant>>,
    /// matcher IRI → accept-sets (principal space), for exceptMatcher evaluation.
    matcher_agents: FxHashMap<String, FxHashSet<String>>,
    matcher_clients: FxHashMap<String, FxHashSet<String>>,
    /// [OPUS-4.8] sq-3jtd.6 — the issuer-dimension twin of `matcher_clients`.
    matcher_issuers: FxHashMap<String, FxHashSet<String>>,
}

impl AuthIndex {
    /// Build from the dataset's `<urn:sparq:auth>` graph (empty index if absent —
    /// fail-closed).
    ///
    /// Only `auth:`-namespace triples (and the `solidx:` matcher accept-set facts the
    /// materializer copies alongside conditional grants) are interpreted; anything
    /// else in the graph is ignored. See [`AuthIndex`] for an end-to-end example.
    pub fn from_graph(graph: &Graph) -> AuthIndex {
        let mut ix = AuthIndex::default();
        let auth_name = Term::NamedNode(NamedNode::new_unchecked(AUTH_GRAPH));
        let Some((_, sub)) = graph.named.iter().find(|(n, _)| *n == auth_name) else {
            return ix;
        };
        let mut cond: FxHashMap<String, ConditionalGrant> = FxHashMap::default();
        for t in graph_triples(sub) {
            let Term::NamedNode(p) = &t[1] else { continue };
            let subj = match &t[0] {
                Term::NamedNode(n) => n.as_str().to_owned(),
                _ => continue,
            };
            if let Some((mode, is_allow)) = Mode::from_pred(p.as_str()) {
                if let Term::NamedNode(g) = &t[2] {
                    let map = if is_allow { &mut ix.allow } else { &mut ix.deny };
                    // [OPUS-4.8] sq-b7k7u: bucket the grant under its graph's origin.
                    let origin = crate::loader::iri_origin(g.as_str()).to_owned();
                    map.entry((subj, mode)).or_default().entry(origin).or_default().push(g.clone());
                }
                continue;
            }
            let Some(local) = p.as_str().strip_prefix(AUTH_NS) else {
                // solidx matcher accept-set facts
                match p.as_str() {
                    "https://sparq.dev/ns/solidx#acceptsAgentP" => {
                        if let Term::NamedNode(o) = &t[2] {
                            ix.matcher_agents.entry(subj).or_default().insert(o.as_str().to_owned());
                        }
                    }
                    "https://sparq.dev/ns/solidx#acceptsClientP" => {
                        if let Term::NamedNode(o) = &t[2] {
                            ix.matcher_clients.entry(subj).or_default().insert(o.as_str().to_owned());
                        }
                    }
                    // [OPUS-4.8] sq-3jtd.6: the issuer-dimension matcher accept-set fact.
                    "https://sparq.dev/ns/solidx#acceptsIssuerP" => {
                        if let Term::NamedNode(o) = &t[2] {
                            let set = ix.matcher_issuers.entry(subj).or_default();
                            set.insert(o.as_str().to_owned());
                        }
                    }
                    _ => {}
                }
                continue;
            };
            let entry = cond.entry(subj).or_default();
            match (local, &t[2]) {
                ("effect", Term::NamedNode(o)) => entry.allow = o.as_str() == format!("{AUTH_NS}Allow"),
                ("agent", Term::NamedNode(o)) => entry.agent = o.as_str().to_owned(),
                ("client", Term::NamedNode(o)) => entry.client = o.as_str().to_owned(),
                // [OPUS-4.8] sq-3jtd.6: the conditional grant's issuer head.
                ("issuer", Term::NamedNode(o)) => entry.issuer = o.as_str().to_owned(),
                ("mode", Term::NamedNode(o)) => entry.mode = Mode::from_mode_iri(o.as_str()),
                ("graph", Term::NamedNode(o)) => entry.graph = Some(o.clone()),
                ("exceptMatcher", Term::NamedNode(o)) => entry.except.push(o.as_str().to_owned()),
                // [OPUS-4.8] sq-0q7n: the live-clock window bounds (xsd:dateTime literals).
                ("notBefore", Term::Literal(o)) => entry.not_before = Some(o.value().to_owned()),
                ("notAfter", Term::Literal(o)) => entry.not_after = Some(o.value().to_owned()),
                _ => {}
            }
        }
        // [OPUS-4.8] sq-b7k7u: bucket each conditional grant under its target graph's
        // origin (empty-origin key when it has no target graph — such a grant is inert).
        for g in cond.into_values() {
            let origin = g.graph.as_ref().map(|n| crate::loader::iri_origin(n.as_str()).to_owned()).unwrap_or_default();
            ix.cond.entry(origin).or_default().push(g);
        }
        ix
    }

    /// The session's agent-dimension principals (most-specific last).
    fn agent_principals(s: &Session) -> Vec<String> {
        let mut ps = vec![PUBLIC.to_owned()];
        if let Some(a) = s.agent {
            ps.push(AUTHENTICATED.to_owned());
            ps.push(a.to_owned());
        }
        ps
    }

    /// All principals the session matches: the agent-dimension principals, plus, for each
    /// one, the minted `(agent, client)` pair (when a client is given), the minted
    /// `(agent, client, issuer)` triple (when an issuer is given — with `auth:AnyClient`
    /// as the client component if no client was supplied), and the full triple. [OPUS-4.8]
    /// sq-3jtd.6: ≤12 lookups (the pre-issuer ≤6 doubled by the issuer dimension).
    fn principals(s: &Session) -> Vec<String> {
        let agents = Self::agent_principals(s);
        let mut ps = agents.clone();
        if let Some(c) = s.client {
            for a in &agents {
                ps.push(pair_principal(a, c));
            }
        }
        // The issuer dimension mints a triple principal. A grant that constrains the
        // issuer but not the client mints `triple(agent, auth:AnyClient, issuer)`, so the
        // session must match that term too — hence the `auth:AnyClient` client component
        // when the session has no client.
        if let Some(i) = s.issuer {
            let client = s.client.unwrap_or(ANY_CLIENT);
            for a in &agents {
                ps.push(triple_principal(a, client, i));
                if s.client.is_some() {
                    ps.push(triple_principal(a, ANY_CLIENT, i));
                }
            }
        }
        ps
    }

    /// Does `matcher` accept this session? (Used for ACP noneOf exceptions: an
    /// accepting exception matcher suppresses the conditional grant.)
    fn matcher_accepts(&self, matcher: &str, s: &Session) -> bool {
        let agent_ok = match self.matcher_agents.get(matcher) {
            None => false, // no accept-set materialized -> cannot accept
            Some(set) => {
                set.contains(PUBLIC)
                    || s.agent
                        .map(|a| set.contains(AUTHENTICATED) || set.contains(a))
                        .unwrap_or(false)
            }
        };
        let client_ok = match self.matcher_clients.get(matcher) {
            None => false,
            Some(set) => {
                set.contains(ANY_CLIENT) || s.client.map(|c| set.contains(c)).unwrap_or(false)
            }
        };
        // [OPUS-4.8] sq-3jtd.6: the issuer-dimension twin of the client check.
        let issuer_ok = match self.matcher_issuers.get(matcher) {
            None => false,
            Some(set) => {
                set.contains(ANY_ISSUER) || s.issuer.map(|i| set.contains(i)).unwrap_or(false)
            }
        };
        agent_ok && client_ok && issuer_ok
    }

    /// Does a conditional grant's (agent, client, issuer) head apply to this session,
    /// AND — for a time-windowed grant — does the session's clock fall inside the window?
    /// [OPUS-4.8] sq-3jtd.6: the issuer head is the twin of the client head — the grant's
    /// `auth:AnyIssuer` matches any session, else the session's issuer must equal it.
    /// [OPUS-4.8] sq-0q7n: the `auth:notBefore`/`auth:notAfter` window is re-checked
    /// against `Session::now` here, so a lapsed window denies *this request* without
    /// waiting for a ledger refresh.
    fn cond_applies(&self, g: &ConditionalGrant, s: &Session) -> bool {
        let agent_ok = Self::agent_principals(s).contains(&g.agent);
        let client_ok = g.client == ANY_CLIENT || s.client == Some(g.client.as_str());
        let issuer_ok = g.issuer == ANY_ISSUER || s.issuer == Some(g.issuer.as_str());
        let window_ok = window_admits(g.not_before.as_deref(), g.not_after.as_deref(), s.now);
        agent_ok
            && client_ok
            && issuer_ok
            && window_ok
            && !g.except.iter().any(|m| self.matcher_accepts(m, s))
    }

    /// The sorted, deduplicated graph set this session may access in `mode`:
    /// `∪ allow(principals) ∖ ∪ deny(principals)` (deny-overrides across principals),
    /// with conditional grants/denies (ACP `noneOf`) applied when their
    /// (agent, client, issuer) head matches the session and no exception matcher accepts it.
    ///
    /// Never panics and never errors — every unauthorized condition degrades to the
    /// **empty set**:
    ///
    /// - no auth view materialized → empty index → empty set;
    /// - no grant names any of the session's principals → empty set;
    /// - session values inside the reserved principal encoding fail CLOSED — a
    ///   caller-supplied "WebID" like `urn:sparq:pair?agent=…&client=…` must not
    ///   impersonate a minted pair principal (roborev 1727).
    ///
    /// Prefer [`crate::PodStore::accessible`], which memoizes this walk per
    /// (agent, client, issuer, mode) until the next re-materialization.
    pub fn accessible(&self, s: &Session, mode: Mode) -> Vec<NamedNode> {
        self.accessible_impl(s, mode, None)
    }

    /// [OPUS-4.8] sq-b7k7u (issue #1571): [`AuthIndex::accessible`] RESTRICTED to graphs
    /// whose origin (`scheme://authority`) is `origin` — the same fail-closed
    /// `∪ allow ∖ ∪ deny` (+ conditional) computation, but visiting only that origin's
    /// buckets. It is exactly `accessible(s, mode)` filtered to `origin`
    /// (`accessible == ⋃ over origins of accessible_in_origin`), so it lets a per-request
    /// decision and a diff-based session-cache re-derivation pay only the affected origin's
    /// cost instead of the whole store's. Internal to the crate.
    pub(crate) fn accessible_in_origin(&self, s: &Session, mode: Mode, origin: &str) -> Vec<NamedNode> {
        self.accessible_impl(s, mode, Some(origin))
    }

    /// Returns all origin strings present in this index's allow/deny/cond buckets (may
    /// yield duplicates; collect into a set for uniqueness). Used by `reindex_with`'s
    /// diff-based invalidation to enumerate the union of old + new origins. [SONNET-4.6]
    /// sq-b7k7u fix.
    pub(crate) fn origin_keys(&self) -> impl Iterator<Item = &str> {
        let a = self.allow.values().flat_map(|by_o| by_o.keys().map(String::as_str));
        let d = self.deny.values().flat_map(|by_o| by_o.keys().map(String::as_str));
        let c = self.cond.keys().map(String::as_str);
        a.chain(d).chain(c)
    }

    /// Returns `true` iff the allow/deny/cond buckets for `origin` are identical between
    /// `self` and `other` — same set of `(principal, mode, graph_iri)` triples in the
    /// allow and deny maps, and same canonical set of conditional grants.
    ///
    /// Used by `reindex_with`'s diff-based invalidation: if this returns `false` for an
    /// origin, that origin's session-cache slice must be re-derived. [SONNET-4.6]
    /// sq-b7k7u fix — no confinement lemma assumed.
    pub(crate) fn bucket_eq(&self, other: &AuthIndex, origin: &str) -> bool {
        // Compare (principal, mode) → graph sets for allow and deny at this origin.
        fn am_triples(
            map: &FxHashMap<(String, Mode), FxHashMap<String, Vec<NamedNode>>>,
            origin: &str,
        ) -> FxHashSet<(String, Mode, String)> {
            let mut out = FxHashSet::default();
            for ((p, m), by_o) in map {
                if let Some(gs) = by_o.get(origin) {
                    for g in gs {
                        out.insert((p.clone(), *m, g.as_str().to_owned()));
                    }
                }
            }
            out
        }
        if am_triples(&self.allow, origin) != am_triples(&other.allow, origin) {
            return false;
        }
        if am_triples(&self.deny, origin) != am_triples(&other.deny, origin) {
            return false;
        }
        // Compare conditional grants: canonical STRUCTURED key per grant (sorted except
        // list), collected as a set to be order-independent.
        //
        // [FABLE-5] sq-vhhl0 (PR #1585 re-review follow-up): the key was previously a
        // `|`/`,`-joined `format!` string, which is NOT injective — a comma is a legal
        // IRI character, so two distinct except-vectors could canonicalize identically
        // (`["a,b"]` vs `["a", "b"]`), letting `bucket_eq` report two genuinely different
        // conditional-grant sets as equal (an under-invalidation of the session cache;
        // not privilege-escalating — only the ACL author's own write could trigger it,
        // and only towards a state that author previously wrote). A structured tuple is
        // injective BY CONSTRUCTION: there is no separator to smuggle, and it also
        // distinguishes `graph: None` from an empty-IRI target the old string folded
        // into the same `""`. Strictly finer than (or equal to) the old key — equal
        // tuples imply equal fields imply the old strings were equal too — so the change
        // can only ADD cache invalidation, never lose any.
        type CondKey = (
            bool,           // allow
            Option<Mode>,   // mode
            String,         // agent
            String,         // client
            String,         // issuer
            Option<String>, // target graph IRI
            Option<String>, // not_before
            Option<String>, // not_after
            Vec<String>,    // except matcher IRIs, sorted
        );
        fn cond_key(g: &ConditionalGrant) -> CondKey {
            let mut ex = g.except.clone();
            ex.sort_unstable();
            (
                g.allow,
                g.mode,
                g.agent.clone(),
                g.client.clone(),
                g.issuer.clone(),
                g.graph.as_ref().map(|n| n.as_str().to_owned()),
                g.not_before.clone(),
                g.not_after.clone(),
                ex,
            )
        }
        let self_conds: FxHashSet<CondKey> =
            self.cond.get(origin).into_iter().flatten().map(cond_key).collect();
        let other_conds: FxHashSet<CondKey> =
            other.cond.get(origin).into_iter().flatten().map(cond_key).collect();
        self_conds == other_conds
    }

    /// Returns `true` iff `matcher_agents`, `matcher_clients`, and `matcher_issuers` are
    /// identical between `self` and `other`. These matcher maps are **not** origin-bucketed:
    /// a change in any matcher can affect conditional-grant outcomes in any origin, so a
    /// matcher difference triggers full cache invalidation in `reindex_with` (the
    /// diff-based approach falls back to `Full` when matchers change). [SONNET-4.6]
    /// sq-b7k7u fix.
    pub(crate) fn matchers_eq(&self, other: &AuthIndex) -> bool {
        self.matcher_agents == other.matcher_agents
            && self.matcher_clients == other.matcher_clients
            && self.matcher_issuers == other.matcher_issuers
    }

    /// Whether the auth view explicitly denies `mode` on `graph` to this session: a deny
    /// triple naming one of its principals, or an applicable conditional deny. Fails
    /// closed: a session value inside the reserved principal encoding counts as denied.
    #[cfg(feature = "odrl-bridge")]
    pub(crate) fn denies(&self, s: &Session, mode: Mode, graph: &str) -> bool {
        let invalid = |v: Option<&str>| v.is_some_and(|x| !crate::loader::session_value_allowed(x));
        if invalid(s.agent) || invalid(s.client) || invalid(s.issuer) {
            return true;
        }
        let origin = crate::loader::iri_origin(graph);
        let named = |gs: &Vec<NamedNode>| gs.iter().any(|g| g.as_str() == graph);
        Self::principals(s).iter().any(|p| {
            self.deny
                .get(&(p.clone(), mode))
                .and_then(|by_o| by_o.get(origin))
                .is_some_and(named)
        }) || self.cond.get(origin).into_iter().flatten().any(|c| {
            !c.allow
                && c.mode == Some(mode)
                && c.graph.as_ref().is_some_and(|g| g.as_str() == graph)
                && self.cond_applies(c, s)
        })
    }

    /// The shared accessible walk. `only == Some(o)` restricts every lookup to origin `o`'s
    /// bucket (a per-origin decision); `only == None` unions all buckets — byte-for-byte
    /// the pre-partition audited result. [OPUS-4.8] sq-b7k7u.
    fn accessible_impl(&self, s: &Session, mode: Mode, only: Option<&str>) -> Vec<NamedNode> {
        let invalid = |v: Option<&str>| v.is_some_and(|x| !crate::loader::session_value_allowed(x));
        // [OPUS-4.8] sq-3jtd.6: the issuer is a triple-principal ingredient too — a session
        // issuer inside the reserved space could otherwise impersonate a minted triple.
        if invalid(s.agent) || invalid(s.client) || invalid(s.issuer) {
            return Vec::new();
        }
        let principals = Self::principals(s);
        let mut allowed: FxHashSet<NamedNode> = FxHashSet::default();
        let mut denied: FxHashSet<NamedNode> = FxHashSet::default();
        for p in &principals {
            if let Some(by_o) = self.allow.get(&(p.clone(), mode)) {
                extend_from_buckets(&mut allowed, by_o, only);
            }
            if let Some(by_o) = self.deny.get(&(p.clone(), mode)) {
                extend_from_buckets(&mut denied, by_o, only);
            }
        }
        // Conditional (ACP noneOf) grants: all buckets when unrestricted, else this
        // origin's bucket only. An origin-restricted grant only ever inserts a graph in
        // that origin, so restricting the visited buckets never changes the result.
        let conds: Box<dyn Iterator<Item = &ConditionalGrant>> = match only {
            None => Box::new(self.cond.values().flatten()),
            Some(o) => Box::new(self.cond.get(o).into_iter().flatten()),
        };
        for c in conds {
            if c.mode == Some(mode) && self.cond_applies(c, s) {
                if let Some(g) = &c.graph {
                    if c.allow {
                        allowed.insert(g.clone());
                    } else {
                        denied.insert(g.clone());
                    }
                }
            }
        }
        let mut out: Vec<NamedNode> = allowed.into_iter().filter(|g| !denied.contains(g)).collect();
        out.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
        out
    }
}

/// Collect the graphs of a (principal, mode) origin-bucketed grant map into `sink`:
/// every origin's graphs when `only == None`, else just origin `o`'s. [OPUS-4.8] sq-b7k7u.
fn extend_from_buckets(
    sink: &mut FxHashSet<NamedNode>,
    by_origin: &FxHashMap<String, Vec<NamedNode>>,
    only: Option<&str>,
) {
    match only {
        None => by_origin.values().for_each(|gs| sink.extend(gs.iter().cloned())),
        Some(o) => {
            if let Some(gs) = by_origin.get(o) {
                sink.extend(gs.iter().cloned());
            }
        }
    }
}

/// Does the live clock `now` fall inside a conditional grant's
/// `[not_before, not_after]` window (both bounds inclusive)? [OPUS-4.8] sq-0q7n.
///
/// - A grant with NO window (`not_before == None && not_after == None`) admits any
///   session, including one with `now == None` — there is no clock dimension to check.
/// - A windowed grant evaluated with `now == None` **fails closed**: no clock evidence
///   means we cannot prove the request falls inside the window (mirrors the ODRL
///   evaluator's fail-closed `dateTime` constraint with no request-context value).
/// - Bounds compare by the **real UTC instant** each `xsd:dateTime` denotes — the SAME
///   offset-aware semantics `sparq_policy`'s evaluator uses for `odrl:dateTime`
///   (`parse_instant().cmp()`, sq-qj2q), NOT raw lexical `str::cmp`. A mixed-offset
///   request such as `now = 2026-06-16T11:00:00-02:00` (real instant `13:00Z`) is
///   correctly seen as *after* a `not_after = 2026-06-16T12:00:00Z` window and DENIED,
///   even though lexically `"…11…" < "…12…"`. A window emitted from an ODRL constraint
///   therefore re-checks **identically** to how the constraint itself evaluated, closing
///   the lexical fail-open. **An unparseable bound or clock fails closed** (the instant
///   comparison is undefined → we cannot prove the request is inside the window → deny),
///   mirroring the evaluator's `None`-is-fail-closed contract.
fn window_admits(not_before: Option<&str>, not_after: Option<&str>, now: Option<&str>) -> bool {
    use std::cmp::Ordering;
    if not_before.is_none() && not_after.is_none() {
        return true; // no window → the clock is irrelevant.
    }
    let Some(now) = now else {
        return false; // windowed grant, no clock evidence → fail-closed.
    };
    if let Some(nb) = not_before {
        // now must be at-or-after the open instant; unparseable / before → deny.
        match cmp_datetime_instant(now, nb) {
            Some(Ordering::Greater | Ordering::Equal) => {}
            _ => return false,
        }
    }
    if let Some(na) = not_after {
        // now must be at-or-before the close instant; unparseable / after → deny.
        match cmp_datetime_instant(now, na) {
            Some(Ordering::Less | Ordering::Equal) => {}
            _ => return false,
        }
    }
    true
}

/// Order two `xsd:dateTime` lexical forms by the **real UTC instant** they denote
/// (offset-aware), returning `None` when either is unparseable — the single source of
/// truth for the window re-check's dateTime comparison. [OPUS-4.8] sq-0q7n.
///
/// With the `odrl-bridge` feature this delegates straight to
/// [`sparq_policy::cmp_datetime`], so the window enforces on byte-for-byte the same
/// instant normalizer the ODRL evaluator used when it emitted the bound — there is no
/// second dateTime parser in play. Without the feature (the lean default build carries
/// no `sparq-policy` dependency, yet an externally-loaded AUTH_GRAPH can still carry
/// `auth:notBefore`/`auth:notAfter`), it uses the std-only [`cmp_datetime_instant_local`]
/// twin, which the `odrl-bridge`-gated `parity_with_sparq_policy_evaluator` test pins to
/// the canonical answers so the two builds can never diverge.
fn cmp_datetime_instant(x: &str, y: &str) -> Option<std::cmp::Ordering> {
    #[cfg(feature = "odrl-bridge")]
    {
        sparq_policy::cmp_datetime(x, y)
    }
    #[cfg(not(feature = "odrl-bridge"))]
    {
        cmp_datetime_instant_local(x, y)
    }
}

/// Std-only twin of `sparq_policy::cmp_datetime` (instant order, `None` on either operand
/// unparseable). The comparator the lean default build uses; under `odrl-bridge` the
/// runtime path delegates to `sparq_policy::cmp_datetime` instead, so this and its parser
/// chain are compiled there **only** as support for the `parity_with_sparq_policy_evaluator`
/// test that proves the two never drift. [OPUS-4.8] sq-0q7n.
#[cfg(any(not(feature = "odrl-bridge"), test))]
fn cmp_datetime_instant_local(x: &str, y: &str) -> Option<std::cmp::Ordering> {
    Some(parse_utc_instant(x)?.cmp(&parse_utc_instant(y)?))
}

/// A point on the UTC timeline as `(days-since-epoch, nanoseconds-into-day)`, after
/// applying the lexical form's timezone offset — `Ord` by derive *is* instant ordering.
/// Mirrors `sparq_policy::eval::Instant`. [OPUS-4.8] sq-0q7n.
#[cfg(any(not(feature = "odrl-bridge"), test))]
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct UtcInstant {
    day: i64,
    nanos_in_day: i64,
}

/// Parse an `xsd:dateTime`/`xsd:date` lexical form into a UTC instant (std-only, no
/// `chrono`/`time`). Accepts `YYYY-MM-DD[Thh:mm:ss[.frac]][Z|±hh:mm]`; a missing tz is
/// UTC. Returns `None` for anything not matching this grammar (→ fail-closed). This is
/// the feature-off twin of `sparq_policy`'s `parse_instant`. [OPUS-4.8] sq-0q7n.
#[cfg(any(not(feature = "odrl-bridge"), test))]
fn parse_utc_instant(s: &str) -> Option<UtcInstant> {
    let s = s.trim();
    let (date, rest) = match s.split_once('T') {
        Some((d, r)) => (d, Some(r)),
        None => (s, None),
    };
    let (year, month, dom) = parse_ymd(date)?;
    let epoch_day = days_from_civil(year, month, dom)?;
    let Some(rest) = rest else {
        return Some(UtcInstant { day: epoch_day, nanos_in_day: 0 });
    };
    let (time, offset_min) = split_offset(rest)?;
    let (hh, mm, ss, frac_nanos) = parse_hms(time)?;
    let local_nanos = (hh as i64) * 3_600_000_000_000
        + (mm as i64) * 60_000_000_000
        + (ss as i64) * 1_000_000_000
        + frac_nanos;
    let utc_nanos = local_nanos - (offset_min as i64) * 60_000_000_000;
    Some(UtcInstant {
        day: epoch_day + utc_nanos.div_euclid(86_400_000_000_000),
        nanos_in_day: utc_nanos.rem_euclid(86_400_000_000_000),
    })
}

#[cfg(any(not(feature = "odrl-bridge"), test))]
fn parse_ymd(s: &str) -> Option<(i64, u32, u32)> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let mut it = body.splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let mo_s = it.next()?;
    let d_s = it.next()?;
    if it.next().is_some() || mo_s.len() != 2 || d_s.len() != 2 {
        return None;
    }
    let month: u32 = mo_s.parse().ok()?;
    let dom: u32 = d_s.parse().ok()?;
    if !(1..=12).contains(&month) || dom < 1 || dom > days_in_month(y, month)? {
        return None;
    }
    Some((if neg { -y } else { y }, month, dom))
}

#[cfg(any(not(feature = "odrl-bridge"), test))]
fn split_offset(rest: &str) -> Option<(&str, i32)> {
    if let Some(t) = rest.strip_suffix('Z') {
        return Some((t, 0));
    }
    // Find the sign of the offset (after the time-of-day, so skip a leading position).
    for (i, c) in rest.char_indices().skip(1) {
        if c == '+' || c == '-' {
            let (time, off) = rest.split_at(i);
            let sign = if c == '+' { 1 } else { -1 };
            let off = &off[1..];
            let (h, m) = off.split_once(':')?;
            if h.len() != 2 || m.len() != 2 {
                return None;
            }
            let hh: i32 = h.parse().ok()?;
            let mm: i32 = m.parse().ok()?;
            if hh > 14 || mm > 59 || (hh == 14 && mm != 0) {
                return None;
            }
            return Some((time, sign * (hh * 60 + mm)));
        }
    }
    Some((rest, 0)) // no tz → UTC
}

#[cfg(any(not(feature = "odrl-bridge"), test))]
fn parse_hms(s: &str) -> Option<(u32, u32, u32, i64)> {
    let (hms, frac) = match s.split_once('.') {
        Some((a, f)) => (a, Some(f)),
        None => (s, None),
    };
    let mut it = hms.splitn(3, ':');
    let h_s = it.next()?;
    let m_s = it.next()?;
    let s_s = it.next()?;
    if it.next().is_some() || h_s.len() != 2 || m_s.len() != 2 || s_s.len() != 2 {
        return None;
    }
    let hh: u32 = h_s.parse().ok()?;
    let mm: u32 = m_s.parse().ok()?;
    let ss: u32 = s_s.parse().ok()?;
    if hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    let frac_nanos = match frac {
        None => 0,
        Some(f) if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) => return None,
        Some(f) => {
            let mut padded = String::with_capacity(9);
            padded.push_str(&f[..f.len().min(9)]);
            while padded.len() < 9 {
                padded.push('0');
            }
            padded.parse().ok()?
        }
    };
    Some((hh, mm, ss, frac_nanos))
}

#[cfg(any(not(feature = "odrl-bridge"), test))]
fn days_in_month(year: i64, month: u32) -> Option<u32> {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    })
}

/// Days since 1970-01-01 (Howard Hinnant's proleptic-Gregorian `days_from_civil`).
#[cfg(any(not(feature = "odrl-bridge"), test))]
fn days_from_civil(year: i64, month: u32, dom: u32) -> Option<i64> {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + dom as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod window_tests {
    //! [OPUS-4.8] sq-0q7n — soundness of the live-clock window predicate. The
    //! integration suite (`tests/odrl_bridge.rs`) covers the `lteq`/`notAfter` path
    //! end-to-end; these unit tests pin the `gteq`/`notBefore`, two-sided, and
    //! fail-closed corner cases directly.
    use super::window_admits;

    const T0: &str = "2026-01-01T00:00:00Z";
    const T1: &str = "2026-06-17T00:00:00Z";
    const T2: &str = "2026-12-31T00:00:00Z";

    #[test]
    fn no_window_admits_anything_including_no_clock() {
        assert!(window_admits(None, None, None));
        assert!(window_admits(None, None, Some(T1)));
    }

    #[test]
    fn windowed_with_no_clock_fails_closed() {
        assert!(!window_admits(None, Some(T2), None), "notAfter + no clock → deny");
        assert!(!window_admits(Some(T0), None, None), "notBefore + no clock → deny");
        assert!(!window_admits(Some(T0), Some(T2), None), "two-sided + no clock → deny");
    }

    #[test]
    fn not_after_is_inclusive_upper_bound() {
        assert!(window_admits(None, Some(T2), Some(T1)), "before close → admit");
        assert!(window_admits(None, Some(T2), Some(T2)), "exactly at close → admit (inclusive)");
        assert!(!window_admits(None, Some(T0), Some(T1)), "after close → deny");
    }

    #[test]
    fn not_before_is_inclusive_lower_bound() {
        assert!(window_admits(Some(T0), None, Some(T1)), "after open → admit");
        assert!(window_admits(Some(T0), None, Some(T0)), "exactly at open → admit (inclusive)");
        assert!(!window_admits(Some(T2), None, Some(T1)), "before open → deny");
    }

    #[test]
    fn two_sided_window_admits_only_inside() {
        assert!(window_admits(Some(T0), Some(T2), Some(T1)), "inside → admit");
        assert!(!window_admits(Some(T0), Some(T2), Some("2025-01-01T00:00:00Z")), "before → deny");
        assert!(!window_admits(Some(T0), Some(T2), Some("2027-01-01T00:00:00Z")), "after → deny");
    }

    // [OPUS-4.8] sq-0q7n — REGRESSION for the access-control fail-open the adversarial
    // verify flagged: the pre-fix `window_admits` compared `now`/bounds with raw lexical
    // `str::cmp`, so a request whose clock is lexically-inside but instant-OUTSIDE the
    // window was wrongly admitted (granting access after the window closed / before it
    // opened). The fix compares by the real UTC instant, matching the ODRL evaluator.

    #[test]
    fn fail_open_mixed_offset_after_close_is_denied() {
        // The verify's concrete repro: notAfter = 12:00Z; now = 11:00-02:00 (= 13:00Z,
        // AFTER the window closed). Lexically "…11…" < "…12…" so the OLD code admitted;
        // by instant the request is past the close → MUST deny.
        let not_after = "2026-06-16T12:00:00Z";
        let now = "2026-06-16T11:00:00-02:00"; // real instant 13:00Z
        assert!(
            !window_admits(None, Some(not_after), Some(now)),
            "mixed-offset clock after the window closed must DENY (was the fail-open)"
        );
    }

    #[test]
    fn fail_open_mixed_offset_before_open_is_denied() {
        // Symmetric lower-bound case: notBefore = 12:00Z; now = 13:00+02:00 (= 11:00Z,
        // BEFORE the window opened). Lexically "…13…" > "…12…" so the OLD code admitted;
        // by instant the request precedes the open → MUST deny.
        let not_before = "2026-06-16T12:00:00Z";
        let now = "2026-06-16T13:00:00+02:00"; // real instant 11:00Z
        assert!(
            !window_admits(Some(not_before), None, Some(now)),
            "mixed-offset clock before the window opened must DENY (symmetric fail-open)"
        );
    }

    #[test]
    fn mixed_offset_genuinely_inside_window_is_admitted() {
        // A mixed-offset clock that really IS inside must still be admitted (the fix is
        // not a blanket deny): notAfter = 12:00Z; now = 09:00-02:00 (= 11:00Z, before close).
        let not_after = "2026-06-16T12:00:00Z";
        let now = "2026-06-16T09:00:00-02:00"; // real instant 11:00Z, inside
        assert!(window_admits(None, Some(not_after), Some(now)), "instant inside → admit");
        // Two-sided, offset clock genuinely inside.
        assert!(
            window_admits(
                Some("2026-06-16T00:00:00Z"),
                Some("2026-06-17T00:00:00Z"),
                Some("2026-06-16T13:00:00+02:00") // = 11:00Z, inside
            ),
            "two-sided instant inside → admit"
        );
    }

    #[test]
    fn unparseable_bound_or_clock_fails_closed() {
        assert!(!window_admits(None, Some("not-a-date"), Some(T1)), "bad notAfter → deny");
        assert!(!window_admits(Some("garbage"), None, Some(T1)), "bad notBefore → deny");
        assert!(!window_admits(None, Some(T2), Some("???")), "bad clock → deny");
    }

    /// [OPUS-4.8] sq-0q7n — the feature-off fallback comparator MUST agree with the
    /// canonical `sparq_policy::cmp_datetime` (the evaluator's instant normalizer) on
    /// the offset-sensitive cases, so the two builds can never diverge.
    #[cfg(feature = "odrl-bridge")]
    #[test]
    fn parity_with_sparq_policy_evaluator() {
        for (a, b) in [
            ("2026-06-16T11:00:00-02:00", "2026-06-16T12:00:00Z"), // 13:00Z vs 12:00Z
            ("2026-06-16T13:00:00+02:00", "2026-06-16T12:00:00Z"), // 11:00Z vs 12:00Z
            ("2026-06-16T12:00:00Z", "2026-06-16T12:00:00Z"),      // equal
            ("2026-06-15T23:00:00-02:00", "2026-06-16T01:00:00Z"), // both 2026-06-16T01:00Z
            ("not-a-date", "2026-06-16T12:00:00Z"),                // None
        ] {
            assert_eq!(
                super::cmp_datetime_instant_local(a, b),
                sparq_policy::cmp_datetime(a, b),
                "fallback diverged from evaluator on ({a}, {b})"
            );
        }
    }
}

#[cfg(test)]
mod origin_partition_tests {
    //! [OPUS-4.8] sq-b7k7u (issue #1571) — the load-bearing equivalence of the
    //! origin-partitioned index: `accessible(s, m)` is byte-for-byte the union over origins
    //! of `accessible_in_origin(s, m, o)`. If this ever diverged, the per-origin decide path
    //! and the scoped session-cache re-assembly would return a set different from a full
    //! rebuild — the exact fail-open/fail-closed hazard the partition must never introduce.
    use super::*;
    use rustc_hash::FxHashSet;
    use sparq_core::Graph;

    /// A two-pod dataset: alice reads all of pod a, bob reads all of pod b, and alice is
    /// denied one graph in pod a — so the grant sets are non-trivial across two origins.
    fn two_pod_index() -> AuthIndex {
        let nq = r#"
<https://a.ex/n1#it> <https://ex.dev/ns#k> "v" <https://a.ex/n1> .
<https://a.ex/n2#it> <https://ex.dev/ns#k> "v" <https://a.ex/n2> .
<https://b.ex/m1#it> <https://ex.dev/ns#k> "v" <https://b.ex/m1> .
<https://a.ex/.acl#o> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://a.ex/.acl> .
<https://a.ex/.acl#o> <http://www.w3.org/ns/auth/acl#default> <https://a.ex/> <https://a.ex/.acl> .
<https://a.ex/.acl#o> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://a.ex/.acl> .
<https://a.ex/.acl#o> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://a.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://b.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/ns/auth/acl#default> <https://b.ex/> <https://b.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/ns/auth/acl#agent> <https://bob.ex/card#me> <https://b.ex/.acl> .
<https://b.ex/.acl#o> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://b.ex/.acl> .
"#;
        let mut g = Graph::load_dataset(nq, "nquads").expect("loads");
        crate::materialize_wac(&mut g).expect("materializes");
        AuthIndex::from_graph(&g)
    }

    /// The set of origins the index mentions in any allow/deny/cond bucket.
    fn all_origins(ix: &AuthIndex) -> FxHashSet<String> {
        let mut os: FxHashSet<String> = FxHashSet::default();
        for by_o in ix.allow.values().chain(ix.deny.values()) {
            os.extend(by_o.keys().cloned());
        }
        os.extend(ix.cond.keys().cloned());
        os
    }

    #[test]
    fn accessible_equals_union_over_origins() {
        let ix = two_pod_index();
        let sessions = [
            Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None },
            Session { agent: Some("https://bob.ex/card#me"), client: None, issuer: None, now: None },
            Session::default(), // anonymous
        ];
        let origins = all_origins(&ix);
        for s in sessions {
            for mode in [Mode::Read, Mode::Write, Mode::Append, Mode::Control] {
                let full: FxHashSet<_> = ix.accessible(&s, mode).into_iter().collect();
                let mut union: FxHashSet<_> = FxHashSet::default();
                for o in &origins {
                    union.extend(ix.accessible_in_origin(&s, mode, o));
                }
                assert_eq!(full, union, "accessible != ⋃ accessible_in_origin for {s:?}/{mode:?}");
                // Each per-origin slice is exactly the full set restricted to that origin.
                for o in &origins {
                    let slice: FxHashSet<_> = ix.accessible_in_origin(&s, mode, o).into_iter().collect();
                    let restricted: FxHashSet<_> =
                        full.iter().filter(|g| crate::loader::iri_origin(g.as_str()) == o).cloned().collect();
                    assert_eq!(slice, restricted, "slice != full∩origin for {o}");
                }
            }
        }
    }

    #[test]
    fn accessible_in_origin_of_absent_origin_is_empty() {
        let ix = two_pod_index();
        let alice = Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
        // alice reads pod a; she has nothing in pod b or a never-seen origin.
        assert!(!ix.accessible_in_origin(&alice, Mode::Read, "https://a.ex").is_empty());
        assert!(ix.accessible_in_origin(&alice, Mode::Read, "https://b.ex").is_empty());
        assert!(ix.accessible_in_origin(&alice, Mode::Read, "https://never.ex").is_empty());
    }
}

#[cfg(test)]
mod bucket_eq_tests {
    //! [FABLE-5] sq-vhhl0 — injectivity of the conditional-grant canonical key used by
    //! `bucket_eq` (the diff-based session-cache invalidation seam, sq-b7k7u). The old
    //! key joined the sorted except-matcher IRIs with `,` inside a formatted string; a
    //! comma is a legal IRI character, so `except: ["urn:m:a,urn:m:b"]` (ONE matcher
    //! whose IRI contains a comma) and `except: ["urn:m:a", "urn:m:b"]` (TWO matchers)
    //! canonicalized to the SAME key — `bucket_eq` then reported two genuinely different
    //! grant sets as equal and `reindex_with` skipped the re-derivation for that origin
    //! (under-invalidation; not privilege-escalating, see the `cond_key` comment). The
    //! structured tuple key must keep them distinct.
    use super::*;

    const ORIGIN: &str = "https://pod.ex";

    /// A fixed conditional grant whose `except` list is the only varying dimension.
    fn grant(except: &[&str]) -> ConditionalGrant {
        ConditionalGrant {
            allow: true,
            agent: "https://alice.ex/card#me".to_owned(),
            client: "auth:AnyClient".to_owned(),
            issuer: "auth:AnyIssuer".to_owned(),
            mode: Some(Mode::Read),
            graph: Some(NamedNode::new("https://pod.ex/notes/n1").expect("valid IRI")),
            except: except.iter().map(|s| (*s).to_owned()).collect(),
            not_before: None,
            not_after: None,
        }
    }

    /// An index whose only content is `grants` in `ORIGIN`'s conditional bucket.
    fn index_with(grants: Vec<ConditionalGrant>) -> AuthIndex {
        let mut cond: FxHashMap<String, Vec<ConditionalGrant>> = FxHashMap::default();
        cond.insert(ORIGIN.to_owned(), grants);
        AuthIndex { cond, ..AuthIndex::default() }
    }

    /// REGRESSION (sq-vhhl0): except bracketing must be distinguished — ONE matcher IRI
    /// containing a comma is NOT the same grant as TWO matchers joined by that comma.
    /// Under the old `ex.join(",")` string key both sides canonicalized identically and
    /// this assertion failed (mutation witness: revert `cond_key` to the string form and
    /// this test goes red).
    #[test]
    fn except_bracketing_is_distinguished() {
        let one_matcher = index_with(vec![grant(&["urn:m:a,urn:m:b"])]);
        let two_matchers = index_with(vec![grant(&["urn:m:a", "urn:m:b"])]);
        assert!(
            !one_matcher.bucket_eq(&two_matchers, ORIGIN),
            "distinct except-vectors (bracketing differs) must compare UNEQUAL — a false \
             equal here means reindex_with under-invalidates the session cache"
        );
    }

    /// Control: the collision fixture compares EQUAL when the grants really are equal —
    /// so `except_bracketing_is_distinguished` fails for the right reason (the key), not
    /// because the fixture differs elsewhere.
    #[test]
    fn identical_grants_compare_equal() {
        let a = index_with(vec![grant(&["urn:m:a", "urn:m:b"])]);
        let b = index_with(vec![grant(&["urn:m:a", "urn:m:b"])]);
        assert!(a.bucket_eq(&b, ORIGIN), "identical conditional grants must compare equal");
    }

    /// Control: canonicalization properties the tuple key must PRESERVE from the old key —
    /// except-list order-independence (the list is sorted before keying) and grant
    /// order-independence (the keys are collected into a set).
    #[test]
    fn except_order_and_grant_order_stay_canonical() {
        let forward = index_with(vec![grant(&["urn:m:a", "urn:m:b"]), grant(&["urn:m:c"])]);
        let reversed = index_with(vec![grant(&["urn:m:c"]), grant(&["urn:m:b", "urn:m:a"])]);
        assert!(
            forward.bucket_eq(&reversed, ORIGIN),
            "except order + grant order must stay canonical"
        );
    }
}
