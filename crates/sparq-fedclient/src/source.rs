//! Source-type abstraction (design §4.1) — **Phase 2**.
//!
//! This module realises the §4.1 *source-type abstraction*: the [`SourceType`] enum
//! (`Endpoint` | `BrTpf` | `Tpf` | `Local`), the per-source [`Capability`] descriptor,
//! the [`FederatedSource`] trait (the sparq analogue of Comunica's `IQuerySource`), and
//! the [`Endpoint`] adapter that wraps a SPARQL-endpoint over a [`Transport`] seam
//! (`fetch(endpoint, query) -> SRJ string`) **behind a default-deny SSRF egress guard**.
//!
//! # What this phase ships (HONEST scope)
//!
//! * **`SourceType` / `Capability` / `FederatedSource` / `SubQuery` / `FedError`** — the
//!   complete §4.1 type surface, statically resolved (a fine-grained capability
//!   descriptor instead of Comunica's coarse runtime "service-description? /
//!   search-form? / totalItems?" negotiation).
//! * **The `Endpoint` adapter** over the engine's transport *seam*: it sends a SPARQL
//!   query string and gets an SRJ body back, exactly like
//!   `sparq-engine`'s `service.rs` `Transport::fetch(endpoint, query) -> String`
//!   (`service.rs:66`). The engine's own `Transport`/`HttpTransport`/`EgressFilterResolver`
//!   are `pub(crate)` (not exported), so this crate re-declares the *same* one-method
//!   seam ([`Transport`]) and owns its own [`EgressGuard`] — the same default-deny SSRF
//!   policy (loopback / RFC1918 / link-local / cloud-metadata / unique-local /
//!   unspecified / CGNAT refused unless explicitly allowlisted), DNS-rebinding-safe
//!   because it vets the *resolved* IP, not the literal host.
//! * **The SSRF guard is on by DEFAULT and default-DENY** ([`EgressGuard::deny_private`]):
//!   a private/loopback endpoint is rejected before any request is issued; a host must be
//!   explicitly allowlisted to re-open it. Tested both ways (allow + deny).
//!
//! # What is STUBBED for later phases (no overclaim)
//!
//! * `discover()` on [`Endpoint`] is a **stub** that returns the "everything an endpoint
//!   can do" [`Capability`] with **no** [`SourceDescriptor`](sparq_fedplan::SourceDescriptor) — the real VoID/SD discovery
//!   (GET `/.well-known/void` + SD, parse via `SourceDescriptor::from_void_nt`, the
//!   client-side SD parser, ASK-probe fallback) is **Phase 1** (`discovery` module).
//! * `execute()` returns the raw SRJ body (after the egress check + transport round-trip)
//!   rather than a streamed `SolutionStream` (the Phase-5 [`stream`] item, not yet
//!   defined); the SRJ→solution parse and the streaming
//!   boundary are **Phase 5** (`stream`/`operators` modules). The transport seam and the
//!   SSRF gate — the load-bearing reuse + safety pieces of §4.1 — are real here.
//! * [`SourceType::Local`] (in-process `Graph` via `sparq-engine` local eval) is wired in
//!   the planner/operators phases; here it is represented in the enum with a capability
//!   of "everything" and a `not-yet-wired` execute stub.
//!
//! # What **Phase 6** adds (brTPF + TPF — bead sq-2qze)
//!
//! Phase 6 turns the [`SourceType::BrTpf`] / [`SourceType::Tpf`] *capability stubs* into
//! **real Triple-Pattern-Fragments adapters** that return a COMPLETE answer for a single
//! triple pattern:
//!
//! * **[`TpfSource`] (plain TPF)** — fetches the fragment(s) for one triple pattern over a
//!   [`FragmentTransport`] seam, follows `hydra:next` pagination to exhaustion, and binds
//!   the matched triples back into the pattern's variables. There is **no** bind-join: a
//!   plain-TPF source shifts every join client-side, so the adapter just materialises the
//!   whole (selective) fragment for the planner to hash-join locally (design §2.1).
//! * **[`BrTpfSource`] (bindings-restricted TPF)** — additionally pushes a block of *at
//!   most `maxMpR`* upstream bindings with each fragment request, so the server returns
//!   only triples that join with at least one attached binding (the standardised brTPF
//!   bind-join, Hartig & Buil-Aranda, ODBASE 2016). The adapter chunks the upstream
//!   bindings into `maxMpR`-sized blocks, issues one fragment request per block (paginated
//!   to exhaustion), and concatenates the per-block matches — a block/bind nested-loop
//!   join that is **complete** by construction (every binding is offered to the server in
//!   exactly one block, and every matching triple comes back).
//! * **Count-metadata cardinality** — both adapters read the fragment's
//!   `hydra:totalItems`/`void:triples` count (the TPF cardinality oracle) and expose it via
//!   [`TpfSource::cardinality`] / [`BrTpfSource::cardinality`] and a one-pattern
//!   [`SourceDescriptor`](sparq_fedplan::SourceDescriptor) from
//!   [`discover()`](FederatedSource::discover), so the planner's
//!   CostFed estimate keys on the *served* count rather than a uniform guess. For brTPF the
//!   count metadata is the **unbound** pattern count (a recall-safe upper bound; the bound
//!   block only narrows it).
//!
//! The wire seam is the [`FragmentTransport`] trait (one method: fetch one fragment page,
//! optionally with an attached binding block, → matched triples + the page's count + the
//! next-page token). It is the TPF analogue of the [`Transport`] SRJ seam and is tested
//! with an in-memory fixture server ([`source` tests]) so the adapters are exercised on the
//! REAL fetch→parse→bind→paginate→bind-join path with **zero** network.
//!
//! The **native HTTP `FragmentTransport`** ([`HttpFragmentTransport`], bead `sq-yzca`) is the
//! production seam: a blocking ureq GET behind the same default-deny SSRF resolver as
//! [`HttpTransport`], that serialises a [`FragPattern`] into the Hydra TPF URI template
//! (`?subject=&predicate=&object=`), attaches a brTPF binding block as the `values` parameter
//! (the server's text wire), follows the `hydra:next` page link to exhaustion, and parses the
//! Turtle/TriG fragment body — splitting the Hydra/VoID control triples (the
//! `hydra:totalItems`/`void:triples` count + the `hydra:next` link) from the data triples
//! (kept only when they match the requested pattern). Native-only (gated out of wasm).
//!
//! `FederatedSource::execute` still returns the SRJ-style `String` body for the
//! [`Endpoint`] adapter; the TPF adapters answer through the typed
//! [`TpfSource::solutions`] / [`BrTpfSource::solutions`] methods (a fragment server speaks
//! triples, not SPARQL-Results-JSON), and their `execute` forwards a clear
//! [`FedError::Unsupported`] pointing the caller at `solutions` — no overclaim, no lossy
//! re-serialisation through SRJ. The interpreter ([`crate::operators`]) bridges this: a
//! `JoinTree` leaf resolving to a TPF/brTPF source is answered through `solutions` (the typed
//! fragment path), not through `execute` (bead `sq-yzca`).
//!
//! [OPUS-4.8] sq-rsxf (epic sq-dnko / sq-3183): Phase-2 source-type abstraction +
//! Endpoint adapter + default-deny SSRF guard. [OPUS-4.8] sq-2qze: Phase-6 brTPF + TPF
//! adapters + count-metadata cardinality. [OPUS-4.8] sq-yzca: native HTTP `FragmentTransport`
//! (ureq + SSRF resolver + Hydra URI-template + Turtle/TriG parse) + interpreter wiring.
//! Flagged for Fable re-review when available.

use std::collections::HashSet;
use std::net::IpAddr;

// ─── Errors ─────────────────────────────────────────────────────────────────────────

/// Failure modes a [`FederatedSource`] can surface. `String`-payload variants mirror the
/// engine's `Transport`-error convention (`service.rs` returns `Result<_, String>`), so an
/// `Endpoint` can forward a transport error verbatim. [OPUS-4.8] sq-rsxf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FedError {
    /// The endpoint IRI could not be parsed / had no host authority to vet.
    BadEndpoint(String),
    /// The SSRF egress guard refused the endpoint (resolved to a private/internal
    /// address and the host is not on the allowlist), or DNS resolution failed.
    EgressRefused(String),
    /// The underlying transport (`fetch`) returned an error (DNS, connect, non-2xx,
    /// malformed body) — the engine's `Transport`-error string, forwarded verbatim.
    Transport(String),
    /// A capability/interface answered through a different entry point (a fragment source's
    /// `execute` points the caller at its typed `solutions` method; `Local` execute is wired
    /// in the planner/operators phases).
    Unsupported(String),
}

impl std::fmt::Display for FedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FedError::BadEndpoint(m) => write!(f, "federated source: bad endpoint: {m}"),
            FedError::EgressRefused(m) => write!(f, "federated source: egress refused: {m}"),
            FedError::Transport(m) => write!(f, "federated source: transport error: {m}"),
            FedError::Unsupported(m) => write!(f, "federated source: unsupported: {m}"),
        }
    }
}

impl std::error::Error for FedError {}

// ─── The sub-query a source is asked to answer ──────────────────────────────────────

/// The most-precise sub-query the planner pushes to one source: a SPARQL query string
/// plus the variables the caller expects projected back.
///
/// Phase 2 carries the rendered SPARQL text directly (the `Endpoint` adapter forwards it
/// to the transport unchanged — exactly how `sparq-engine`'s SERVICE path wraps an inner
/// pattern as `SELECT * WHERE { … }` before `fetch`). The capability-aware *construction*
/// of this sub-query (projection, pushable FILTERs, VALUES bind-join blocks, ORDER/LIMIT)
/// is the `pushdown` module's job in **Phase 4**; this type is the seam between that phase
/// and the adapters. [OPUS-4.8] sq-rsxf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubQuery {
    /// The SPARQL query string to send to the source (already capability-narrowed by the
    /// time Phase 4 produces it; in Phase 2 it is whatever the caller hands in).
    pub sparql: String,
    /// The projected variable names the caller expects back (without the leading `?`),
    /// used by later phases to bind the SRJ rows; kept here so the adapter surface is
    /// stable. Empty = "whatever the query projects".
    pub project: Vec<String>,
}

impl SubQuery {
    /// A sub-query that forwards `sparql` verbatim with no explicit projection hint.
    pub fn new(sparql: impl Into<String>) -> Self {
        SubQuery {
            sparql: sparql.into(),
            project: Vec::new(),
        }
    }
}

// ─── Capability descriptor (design §4.1) ────────────────────────────────────────────

/// The concrete remote interface a source speaks. Drives which sub-query shape the
/// pushdown layer may build and how the operators bind-join into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    /// A full SPARQL 1.1 (Protocol) endpoint — arbitrary SPARQL, VALUES bind-join.
    Endpoint,
    /// A bindings-restricted Triple Pattern Fragments server — single triple pattern +
    /// a bound-tuple block (`maxMpR(n)`) bind-join. Real adapter in Phase 6 ([`BrTpfSource`]).
    BrTpf,
    /// A plain Triple Pattern Fragments server — single triple pattern, no bind-join.
    /// Real adapter in Phase 6 ([`TpfSource`]).
    Tpf,
    /// The in-process `sparq-engine` over a local `Graph` — "everything".
    LocalEngine,
}

/// How a source supports a bind-join (pushing a block of upstream bindings so the source
/// returns only rows that can survive the local join). Mirrors `sparq-engine`'s SERVICE
/// `VALUES` pushdown (`service.rs`, bead sq-sjkj).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindJoin {
    /// SPARQL `VALUES (…) { … }` pushdown (full endpoints).
    Values,
    /// brTPF `maxMpR(n)` — at most `n` bound mappings per request. (Phase 6.)
    MaxMpR(u32),
    /// No bind-join — the source is materialised and joined locally (plain TPF).
    None,
}

/// Which FILTER expressions a source can evaluate remotely. Coarse classes for now; the
/// pushdown layer (Phase 4) refines per-conjunct with the common-variable check. Only the
/// classes a source *provably* evaluates identically to local eval may be pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterClass {
    /// No FILTER pushdown — every filter is kept local.
    None,
    /// Only simple equality / `IN` over bound terms (safe across most sources).
    Equality,
    /// Full SPARQL 1.1 expression evaluation (assumed for a conformant `Endpoint`).
    Full,
}

/// What a source can do — the fine-grained, statically-resolved capability descriptor
/// (design §4.1, "far richer than Comunica's coarse model").
///
/// Phase 2 fills this from a *static default per interface* (see [`Capability::endpoint`]
/// / [`Capability::local`] / [`Capability::tpf`] / [`Capability::brtpf`]); the
/// `discovery` module (Phase 1) will refine an `Endpoint`'s capability from its parsed
/// Service Description (`sd:supportedLanguage`, `sd:resultFormat`, …). [OPUS-4.8] sq-rsxf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    /// The remote interface this capability describes.
    pub interface: Interface,
    /// Which FILTER class evaluates remotely.
    pub pushable_filters: FilterClass,
    /// The source's bind-join mode.
    pub bind_join: BindJoin,
    /// Can aggregates (`GROUP BY` / `COUNT` / …) be pushed?
    pub aggregates: bool,
    /// Can property-path expressions be pushed?
    pub property_paths: bool,
    /// Can `ORDER BY` / `LIMIT` be pushed?
    pub order_limit: bool,
    /// Advertised result media types (from SD `sd:resultFormat` once discovery lands).
    /// Phase 2 seeds the SRJ default that the transport already negotiates.
    pub result_formats: Vec<String>,
}

impl Capability {
    /// "Everything a conformant SPARQL 1.1 endpoint can do" — the Phase-2 default until
    /// the `discovery` module refines it from the endpoint's Service Description.
    pub fn endpoint() -> Self {
        Capability {
            interface: Interface::Endpoint,
            pushable_filters: FilterClass::Full,
            bind_join: BindJoin::Values,
            aggregates: true,
            property_paths: true,
            order_limit: true,
            result_formats: vec!["application/sparql-results+json".to_string()],
        }
    }

    /// The local in-process engine: everything, no remote transport.
    pub fn local() -> Self {
        Capability {
            interface: Interface::LocalEngine,
            ..Capability::endpoint()
        }
    }

    /// A brTPF source: single triple pattern + `maxMpR` bind-join, no remote FILTER /
    /// aggregate / path / ORDER-LIMIT pushdown. Used by the Phase-6 [`BrTpfSource`] adapter.
    pub fn brtpf(max_mpr: u32) -> Self {
        Capability {
            interface: Interface::BrTpf,
            pushable_filters: FilterClass::None,
            bind_join: BindJoin::MaxMpR(max_mpr),
            aggregates: false,
            property_paths: false,
            order_limit: false,
            result_formats: vec!["application/trig".to_string()],
        }
    }

    /// A plain TPF source: single triple pattern, no bind-join, nothing pushable.
    /// Used by the Phase-6 [`TpfSource`] adapter.
    pub fn tpf() -> Self {
        Capability {
            interface: Interface::Tpf,
            bind_join: BindJoin::None,
            ..Capability::brtpf(0)
        }
    }
}

// ─── The transport seam (REUSE of `sparq-engine`'s `service.rs` Transport) ──────────

/// The HTTP round-trip seam: send a SPARQL `query` string to `endpoint` and get the raw
/// response body (SPARQL-Results-JSON) back, or a transport-error string.
///
/// This is the *same* one-method seam as `sparq-engine`'s `service.rs` `Transport`
/// (`service.rs:66`, `fn fetch(&self, endpoint: &str, query: &str) -> Result<String,
/// String>`). The engine's trait + its `HttpTransport` impl are `pub(crate)` and so cannot
/// be imported; re-declaring the identical shape here lets the `Endpoint` adapter wrap any
/// transport — including a test double — without depending on engine internals, while the
/// dependency arrow still points one-way *into* the engine. A native `HttpTransport`
/// (ureq, default-deny SSRF resolver) lands alongside `execute()`'s streaming in a later
/// phase; Phase 2 ships the seam + the egress gate that fronts it. [OPUS-4.8] sq-rsxf.
pub trait Transport: Send + Sync {
    /// Send `query` to `endpoint`; return the raw response body or an error string.
    fn fetch(&self, endpoint: &str, query: &str) -> Result<String, String>;
}

// ─── SSRF egress guard (default-deny; mirrors the engine's `is_forbidden_ip`) ───────

/// Classifies a resolved [`IpAddr`] as a forbidden (private / internal / non-global)
/// federation destination. This is the **same default-deny classification** as
/// `sparq-engine`'s `service.rs::is_forbidden_ip` (which is `pub(crate)` and cannot be
/// imported): loopback, RFC1918, link-local (incl. the `169.254.169.254` cloud-metadata
/// IP), unique-local IPv6, unspecified, broadcast, and CGNAT (`100.64/10`) are refused;
/// IPv4-mapped IPv6 is unwrapped and re-classified so a private v4 cannot ride in through
/// a v6 literal. Returns `true` when the address must be refused. [OPUS-4.8] sq-rsxf.
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()                 // 127.0.0.0/8
                || v4.is_private()           // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()        // 169.254.0.0/16 (incl. 169.254.169.254)
                || v4.is_unspecified()       // 0.0.0.0
                || v4.is_broadcast()         // 255.255.255.255
                || matches!(v4.octets(), [100, b, ..] if (64..=127).contains(&b))
            // 100.64/10 CGNAT
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_forbidden_ip(IpAddr::V4(v4));
            }
            v6.is_loopback()                              // ::1
                || v6.is_unspecified()                    // ::
                || (v6.segments()[0] & 0xffc0) == 0xfe80  // fe80::/10 link-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        }
    }
}

/// The SSRF egress policy the [`Endpoint`] adapter enforces **before** any request is
/// issued — the client-side analogue of the engine's `with_service_egress_allow`
/// default-deny `EgressFilterResolver` (which is `pub(crate)`).
///
/// **Default-deny by construction.** [`EgressGuard::deny_private`] (the [`Default`]) is
/// the secure default: an endpoint whose host resolves to *any* private/internal address
/// is refused, and a deployer must explicitly allowlist a host to re-open it. This is
/// DNS-rebinding-safe: [`EgressGuard::check_addr`] vets a *resolved* [`IpAddr`], not the
/// literal IRI host, so a hostile DNS answer pointing at loopback / cloud-metadata is
/// dropped on the resolved IP. [OPUS-4.8] sq-rsxf.
#[derive(Debug, Clone, Default)]
pub struct EgressGuard {
    /// Hosts (DNS name or IP literal, lowercased authority) exempt from the
    /// private-address refusal — resolved addresses for these are permitted even when
    /// private. Only ever *adds* permission.
    allow: HashSet<String>,
}

impl EgressGuard {
    /// The secure default: public addresses allowed, private/internal denied unless the
    /// host is allowlisted. Equivalent to [`EgressGuard::default`].
    pub fn deny_private() -> Self {
        EgressGuard {
            allow: HashSet::new(),
        }
    }

    /// Allowlist an entry so a matching host's resolved addresses are permitted even if private.
    /// Chainable. An entry is either:
    ///   * **host-level** (`"sparql.internal"`, `".example.org"` suffix wildcard, `"127.0.0.1"`,
    ///     a bare IPv6 literal `"::1"`) — permits that host on EVERY port (the original meaning,
    ///     preserved for backward compatibility); or
    ///   * **port-scoped** (`"127.0.0.1:8053"`, `".example.org:443"`, bracketed IPv6 `"[::1]:8080"`)
    ///     — permits that host ONLY on the exact `:port`, rejecting every other port on the same
    ///     host. This is strictly narrower; there is no wildcard port and no global bypass.
    ///
    /// The `host:port` split + matching is the engine SERVICE guard's shared rule
    /// ([`sparq_engine::allowlist_entry_permits`]) so the two guards agree on every edge case
    /// (port-0/overflow/IPv6-bracket/trailing-colon all fail-CLOSED). [OPUS-4.8] sq-vbnyc.
    pub fn allow_host(mut self, host: impl Into<String>) -> Self {
        self.allow.insert(host.into().to_ascii_lowercase());
        self
    }

    /// Is `host` permitted on **any** port by the allowlist? Backward-compatible host-level query:
    /// `true` when some entry permits `host` on at least one port — a host-level entry (all ports)
    /// or a port-scoped entry (its single port). The load-bearing, port-precise check is
    /// [`is_allowed_port`](Self::is_allowed_port); this convenience form answers "is this host on
    /// the allowlist at all" and is used where the port is not yet known. [OPUS-4.8] sq-vbnyc.
    pub fn is_allowed(&self, host: &str) -> bool {
        let h = host.to_ascii_lowercase();
        // "Allowed on SOME port" = some entry's host part names `h` (port ignored). Reuses the
        // engine's shared host-pattern parse so bracket-stripping + suffix wildcard are not
        // re-derived here.
        self.allow
            .iter()
            .any(|entry| sparq_engine::allowlist_entry_host_matches(entry, &h))
    }

    /// Is `(host, port)` permitted by the allowlist? The load-bearing, PORT-SCOPED check: a
    /// host-level entry permits `host` on every port; a `host:port` entry permits it ONLY on that
    /// exact port. Delegates to the engine SERVICE guard's shared per-entry rule so the fedclient
    /// guard and the engine guard decide every host:port case identically. [OPUS-4.8] sq-vbnyc.
    pub fn is_allowed_port(&self, host: &str, port: u16) -> bool {
        let h = host.to_ascii_lowercase();
        self.allow
            .iter()
            .any(|entry| sparq_engine::allowlist_entry_permits(entry, &h, port))
    }

    /// The set of allowlisted hosts (lowercased bare authorities), so a native transport can
    /// bridge *this guard's* allowlist into the ureq SSRF resolver — making the IP the guard
    /// vets and the IP the socket dials come from the **same** resolution (no second,
    /// unguarded re-resolve). [OPUS-4.8] sq-25xk.
    pub fn allowed_hosts(&self) -> &HashSet<String> {
        &self.allow
    }

    /// Vet one resolved address for `host` dialled on `port`: `Ok(())` to dial it, `Err(reason)`
    /// to refuse. The allowlist exemption fires only when the host AND port BOTH match (a
    /// port-scoped entry re-opens a private address on its one port only); otherwise a
    /// [`is_forbidden_ip`] address is refused. This is the per-address hook a resolver calls on
    /// every candidate IP, now port-scoped to mirror the engine SERVICE guard. [OPUS-4.8] sq-vbnyc.
    pub fn check_addr(&self, host: &str, port: u16, ip: IpAddr) -> Result<(), String> {
        if self.is_allowed_port(host, port) || !is_forbidden_ip(ip) {
            Ok(())
        } else {
            Err(format!(
                "host {host:?} resolved to private/internal address {ip} on port {port} \
                 (default-deny SSRF policy; allowlist the host to permit it)"
            ))
        }
    }

    /// Vet an endpoint IRI end-to-end: parse the host authority + port, resolve it, and refuse if
    /// *every* resolved address is forbidden (and the host:port is not allowlisted). An
    /// allowlisted host:port short-circuits without a DNS lookup. Returns the bare host on
    /// success (handy for logging / the adapter). On a non-allowlisted host this performs a real
    /// DNS resolution and applies [`check_addr`](Self::check_addr) to each address, mirroring the
    /// engine's resolver: refuse before any socket is opened.
    ///
    /// The PORT is the authority's explicit `:port` or the scheme default (the SAME port actually
    /// dialled), so a port-scoped allowlist entry (`host:port`) is honoured exactly as the engine
    /// SERVICE guard honours it: it re-opens the host on that one port only. [OPUS-4.8] sq-vbnyc.
    pub fn check_endpoint(&self, endpoint: &str) -> Result<String, FedError> {
        let (host, port) = endpoint_host_port(endpoint)
            .ok_or_else(|| FedError::BadEndpoint(format!("no host authority in {endpoint:?}")))?;
        // Allowlisted host:port: permitted without a lookup (matches the engine's resolver, which
        // lets an allowlisted host through even if it resolves privately). A port-scoped entry
        // only short-circuits for its exact dialled port.
        if self.is_allowed_port(&host, port) {
            return Ok(host);
        }
        // An IP-literal authority is vetted directly (no DNS).
        if let Ok(ip) = host.parse::<IpAddr>() {
            return self
                .check_addr(&host, port, ip)
                .map(|()| host.clone())
                .map_err(FedError::EgressRefused);
        }
        // A DNS name: resolve and require at least one permitted address. Resolution uses the
        // host with the dialled port so the std resolver returns socket addresses.
        use std::net::ToSocketAddrs;
        let resolved: Vec<IpAddr> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| {
                FedError::EgressRefused(format!("DNS resolution of {host:?} failed: {e}"))
            })?
            .map(|sa| sa.ip())
            .collect();
        if resolved.is_empty() {
            return Err(FedError::EgressRefused(format!(
                "{host:?} resolved to no addresses"
            )));
        }
        if resolved.iter().all(|ip| is_forbidden_ip(*ip)) {
            return Err(FedError::EgressRefused(format!(
                "{host:?} resolves only to private/internal addresses \
                 (default-deny SSRF policy; allowlist the host to permit it)"
            )));
        }
        Ok(host)
    }
}

/// Extract the bare host authority (no scheme, no userinfo, no port, IPv6 brackets
/// stripped, lowercased) from an endpoint IRI. Returns `None` when there is no authority.
/// A tiny tolerant parser so the egress gate has no URL-crate dependency of its own (the
/// crate already pulls `oxrdf`/`spargebra`, not `url`, under `fedclient`). [OPUS-4.8].
fn endpoint_host(endpoint: &str) -> Option<String> {
    endpoint_host_port(endpoint).map(|(host, _port)| host)
}

/// Extract the bare host authority (lowercased, IPv6 brackets stripped) **and** the authority
/// port from an endpoint IRI: the explicit `:port`, or the scheme default (443 for `https`,
/// 80 otherwise — the SAME port that is actually dialled). Returns `None` when there is no host
/// authority. The port is what makes a PORT-SCOPED allowlist entry (`host:port`) decidable, so the
/// fedclient guard matches the engine SERVICE guard's authority handling exactly. [OPUS-4.8] sq-vbnyc.
fn endpoint_host_port(endpoint: &str) -> Option<(String, u16)> {
    // scheme://authority/path?query — take the scheme + what follows "://".
    let (scheme, after_scheme) = endpoint.split_once("://")?;
    // authority ends at the first '/', '?' or '#'.
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // strip userinfo (user:pass@host).
    let hostport = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    if hostport.is_empty() {
        return None;
    }
    let default_port = if scheme.eq_ignore_ascii_case("https") {
        443
    } else {
        80
    };
    // IPv6 literal: [::1] or [::1]:80 — take what is inside the brackets; the port (if any)
    // follows the closing bracket.
    if let Some(rest) = hostport.strip_prefix('[') {
        let (host, after) = rest.split_once(']').unwrap_or((rest, ""));
        if host.is_empty() {
            return None;
        }
        let port = after
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(default_port);
        return Some((host.to_ascii_lowercase(), port));
    }
    // host or host:port — split off the port (only a single trailing `:digits`; a bare IPv6
    // literal here has no brackets but `to_socket_addrs` would reject it anyway, so a multi-colon
    // string keeps the whole thing as the host and the default port).
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, p.parse::<u16>().unwrap_or(default_port)),
        _ => (hostport, default_port),
    };
    (!host.is_empty()).then(|| (host.to_ascii_lowercase(), port))
}

// ─── Native HTTP transport that PINS the guard-vetted IP (no re-resolve TOCTOU) ──────

/// The production [`Transport`]: a blocking ureq POST (SPARQL Protocol §2.1.2 — `query=`
/// form-encoded body, `Accept: application/sparql-results+json`) that **closes the
/// DNS-rebinding TOCTOU re-resolve window** by installing an SSRF [`Resolver`](ureq::unversioned::resolver::Resolver)
/// on the ureq agent itself.
///
/// # Why this exists (bead sq-25xk, follow-up to sq-rsxf)
///
/// Phase 2 shipped the [`Transport`] *seam* + the [`EgressGuard`] but only **in-test
/// transport doubles**; there was no native HTTP `Transport`. A naive native transport would
/// re-open the very window the guard was meant to close: [`Endpoint::execute`] calls
/// [`EgressGuard::check_endpoint`], which resolves the host and vets the IP — but if the
/// transport is an ordinary HTTP client it then **re-resolves the host independently** before
/// connecting, so a hostile DNS server can answer a public IP to the guard's lookup and a
/// private/cloud-metadata IP to the socket lookup (a time-of-check-to-time-of-use SSRF
/// re-bind). The check is on a *different* address than the one dialled.
///
/// [`HttpTransport`] removes that gap exactly as `sparq-engine`'s `service.rs`
/// `HttpTransport` does (`.resolver(EgressFilterResolver)`, `service.rs:692`): the egress
/// policy runs **inside ureq's own resolver**, so ureq connects ONLY to the addresses the
/// resolver returns — the resolved-and-vetted IP IS the dialled IP. There is no second,
/// unguarded re-resolve. The guard's [`allowlist`](EgressGuard::allowed_hosts) is bridged
/// into the resolver, so an allowlisted private host is reachable through BOTH the pre-flight
/// `check_endpoint` and the socket resolver — one source of truth.
///
/// Native-only (`cfg(not(target_arch = "wasm32"))`): neither ureq nor its TLS stack ever
/// enters the wasm bundle (the wasm federation client would carry a `fetch`-based transport
/// instead). Build one with [`Endpoint::native`] / [`Endpoint::with_guard_native`], or
/// directly via [`HttpTransport::new`] / [`HttpTransport::from_guard`]. [OPUS-4.8] sq-25xk.
#[cfg(not(target_arch = "wasm32"))]
pub struct HttpTransport {
    timeout: std::time::Duration,
    /// Hosts (bare authority, lowercased) the SSRF resolver permits even when they resolve
    /// privately — bridged from the [`EgressGuard`] so both the pre-flight check and the
    /// socket resolver share one allowlist.
    allow_private: std::sync::Arc<HashSet<String>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl HttpTransport {
    /// A transport with a finite default timeout (so an unreachable endpoint cannot hang the
    /// client) and the strict default-deny SSRF policy (no private host permitted).
    pub fn new() -> HttpTransport {
        HttpTransport {
            // Mirrors `sparq-engine`'s 30s SERVICE default — generous for a slow endpoint,
            // finite so a black-holed host fails-stop rather than hanging.
            timeout: std::time::Duration::from_secs(30),
            allow_private: std::sync::Arc::new(HashSet::new()),
        }
    }

    /// A transport whose SSRF resolver shares `guard`'s allowlist, so the IP the guard vets in
    /// [`EgressGuard::check_endpoint`] and the IP this transport's socket dials come from the
    /// SAME guarded resolution. This is the bridge that closes the re-resolve TOCTOU window.
    pub fn from_guard(guard: &EgressGuard) -> HttpTransport {
        HttpTransport {
            timeout: std::time::Duration::from_secs(30),
            allow_private: std::sync::Arc::new(guard.allowed_hosts().clone()),
        }
    }

    /// Sets the per-request timeout. Chainable.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> HttpTransport {
        self.timeout = timeout;
        self
    }
}

/// ureq [`Resolver`](ureq::unversioned::resolver::Resolver) that enforces the SSRF egress policy on the *resolved*
/// addresses (DNS-rebinding-safe): it resolves the host, drops every [`is_forbidden_ip`]
/// address (unless the bare host is on the allowlist), and returns only the survivors — so
/// ureq dials only vetted IPs and there is no resolve-then-re-resolve gap. Mirrors the
/// engine's `service.rs` `EgressFilterResolver` and this crate's `discovery::EgressFilterResolver`.
/// [OPUS-4.8] sq-25xk.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct EgressFilterResolver {
    allow_private: std::sync::Arc<HashSet<String>>,
}

// [OPUS-4.8] sq-g2xs: ureq-3's `Resolver` takes a parsed `&http::Uri` (+ config + timeout) and
// returns an `ArrayVec<SocketAddr, 16>` rather than ureq-2's `&str` netloc → `Vec`. The SSRF
// policy is byte-for-byte the ureq-2 one (default-deny private/internal, allowlist-to-reopen,
// DNS-rebinding-safe); the shared `crate::ureq_egress` helpers carry the ureq-3 boilerplate so
// the three sparq-fedclient transports share one implementation. (The engine SERVICE resolver
// applies equivalent, unit-tested logic via its own inline copy — `sparq-engine` cannot depend on
// `sparq-fedclient` — so it does not share this module.)
#[cfg(not(target_arch = "wasm32"))]
impl ureq::unversioned::resolver::Resolver for EgressFilterResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let (host_port, host, port) = crate::ureq_egress::uri_host_port(uri).ok_or_else(|| {
            crate::ureq_egress::egress_refused(format!(
                "federation egress refused: request URI {uri} has no host authority to vet"
            ))
        })?;
        // Port-scoped allowlist: a `host:port` entry re-opens a private host only on its exact
        // dialled port (shared host:port rule with the engine SERVICE guard; sq-vbnyc).
        let allowed = crate::ureq_egress::allowlist_permits(&self.allow_private, &host, port);
        crate::ureq_egress::filter_resolved(&host_port, allowed, is_forbidden_ip, || {
            format!(
                "federation egress refused: {host_port} resolves only to private/internal \
                 addresses (default-deny SSRF policy; allowlist the host on the EgressGuard)"
            )
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Transport for HttpTransport {
    fn fetch(&self, endpoint: &str, query: &str) -> Result<String, String> {
        // [OPUS-4.8] sq-g2xs: ureq-3 builds an `Agent` from a `Config` + a custom resolver via
        // `Agent::with_parts`; the resolver carries the default-deny SSRF policy exactly as in
        // ureq 2 — ureq connects only to the addresses the resolver returns, so the
        // resolved-and-vetted IP IS the dialled IP (no DNS-rebinding re-resolve window). [sq-25xk]
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .user_agent(concat!("sparq-fedclient/", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::new(),
            EgressFilterResolver {
                allow_private: std::sync::Arc::clone(&self.allow_private),
            },
        );
        // SPARQL Protocol §2.1.2: query via POST with `application/x-www-form-urlencoded`
        // `query=` — the broadly-supported method, not subject to URL-length limits. Same
        // shape as `sparq-engine`'s SERVICE transport so a sub-query travels identically.
        match agent
            .post(endpoint)
            .header("Accept", "application/sparql-results+json")
            .send_form([("query", query)])
        {
            // ureq-3 caps `read_to_string` at 10 MiB by default; a federated SELECT result can
            // exceed that, so raise the limit (a finite cap still bounds memory).
            Ok(mut r) => r
                .body_mut()
                .with_config()
                .limit(MAX_BODY_BYTES)
                .read_to_string()
                .map_err(|e| format!("fedclient: reading response from {endpoint}: {e}")),
            // ureq-3 surfaces non-2xx as `Error::StatusCode`; treat transport + HTTP errors uniformly.
            Err(ureq::Error::StatusCode(code)) => Err(format!(
                "fedclient: endpoint {endpoint} returned HTTP {code}"
            )),
            Err(e) => Err(format!("fedclient: request to {endpoint} failed: {e}")),
        }
    }
}

/// Max bytes read from a federation response body. ureq-3's default `read_to_string` cap is
/// 10 MiB; a federated SELECT / TPF fragment can legitimately exceed that, so we raise it to a
/// generous-but-finite bound (memory stays bounded). [OPUS-4.8] sq-g2xs.
#[cfg(not(target_arch = "wasm32"))]
const MAX_BODY_BYTES: u64 = 1024 * 1024 * 1024;

// ─── The FederatedSource trait (design §4.1) ────────────────────────────────────────

/// One remote (or local) RDF source the federation engine can query — the sparq analogue
/// of Comunica's `IQuerySource`.
///
/// `discover()` resolves the source's [`Capability`] (and, once Phase 1 lands, its
/// [`SourceDescriptor`](sparq_fedplan::SourceDescriptor) statistics) one-shot; `execute()` answers the most-precise
/// sub-query the source can evaluate. Phase 2 returns the raw SRJ body from `execute()`;
/// the streamed `SolutionStream` boundary is Phase 5. [OPUS-4.8] sq-rsxf.
pub trait FederatedSource {
    /// Which interface this source speaks — drives pushdown shape and operator choice.
    fn source_type(&self) -> SourceType<'_>;

    /// Discover the source's capability (and statistics, once Phase 1's discovery lands).
    /// Phase 2: returns the static per-interface [`Capability`] with no descriptor.
    fn discover(&self) -> Result<(Capability, Option<sparq_fedplan::SourceDescriptor>), FedError>;

    /// Answer `sub` against this source. Phase 2: vets the endpoint through the SSRF guard,
    /// forwards `sub.sparql` to the transport, and returns the raw SRJ body. The SRJ→
    /// solution parse and the streaming boundary are Phase 5. [OPUS-4.8] sq-rsxf.
    fn execute(&self, sub: &SubQuery) -> Result<String, FedError>;
}

// ─── SourceType — the enum of adapters ──────────────────────────────────────────────

/// The set of source adapters the client fans out to (design §4.1). Borrows the adapter
/// so a planner can hold a heterogeneous `&[SourceType]` without owning the transports.
///
/// * [`SourceType::Endpoint`] — a full SPARQL endpoint over the transport seam (Phase 2,
///   real here);
/// * [`SourceType::BrTpf`] / [`SourceType::Tpf`] — bindings-restricted / plain TPF
///   (**Phase 6, real here**: a fragment-server adapter that returns a complete answer for
///   one triple pattern via [`BrTpfSource::solutions`] / [`TpfSource::solutions`]);
/// * [`SourceType::Local`] — the in-process engine (capability "everything"; `execute`
///   is wired in the planner/operators phases). [OPUS-4.8] sq-rsxf / sq-2qze.
pub enum SourceType<'a> {
    /// A full SPARQL 1.1 endpoint.
    Endpoint(&'a Endpoint),
    /// A bindings-restricted Triple Pattern Fragments server (Phase 6, [`BrTpfSource`]).
    BrTpf(&'a BrTpfSource),
    /// A plain Triple Pattern Fragments server (Phase 6, [`TpfSource`]).
    Tpf(&'a TpfSource),
    /// The in-process `sparq-engine` over a local `Graph`.
    Local(&'a LocalSource),
}

// ─── Endpoint adapter (Phase 2 — the real one) ──────────────────────────────────────

/// A full SPARQL-endpoint source: wraps a [`Transport`] (`fetch(endpoint, query) ->
/// SRJ`) behind a default-deny [`EgressGuard`]. This is the Phase-2 deliverable — the
/// SPARQL-endpoint reuse of the engine's transport seam plus the SSRF safety gate.
///
/// Construct with [`Endpoint::new`] (default-deny SSRF) or [`Endpoint::with_guard`] to
/// supply an allowlisting [`EgressGuard`]. The transport is any `Transport` impl — a real
/// HTTP client or a test double — so the adapter is exercised without a network. The host
/// is **vetted on every `execute`** (not cached), matching the engine's
/// resolve-then-dial-the-vetted-IP discipline. [OPUS-4.8] sq-rsxf.
pub struct Endpoint {
    endpoint: String,
    transport: Box<dyn Transport>,
    guard: EgressGuard,
}

impl Endpoint {
    /// A new endpoint source with the **secure default** SSRF guard (default-deny
    /// private/internal). `endpoint` is the SPARQL Protocol URL; `transport` performs the
    /// round-trip.
    pub fn new(endpoint: impl Into<String>, transport: Box<dyn Transport>) -> Self {
        Endpoint {
            endpoint: endpoint.into(),
            transport,
            guard: EgressGuard::deny_private(),
        }
    }

    /// A new endpoint source with an explicit [`EgressGuard`] (e.g. one that allowlists a
    /// trusted internal host).
    pub fn with_guard(
        endpoint: impl Into<String>,
        transport: Box<dyn Transport>,
        guard: EgressGuard,
    ) -> Self {
        Endpoint {
            endpoint: endpoint.into(),
            transport,
            guard,
        }
    }

    /// A new endpoint source over the native IP-pinning [`HttpTransport`] with the **secure
    /// default** SSRF guard. The transport's ureq resolver and the pre-flight guard share the
    /// (empty) default-deny allowlist, so the IP vetted before the request is the IP dialled —
    /// the DNS-rebinding re-resolve window is closed end-to-end. Native-only. [OPUS-4.8] sq-25xk.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn native(endpoint: impl Into<String>) -> Self {
        Endpoint {
            endpoint: endpoint.into(),
            transport: Box::new(HttpTransport::new()),
            guard: EgressGuard::deny_private(),
        }
    }

    /// A new endpoint source over the native IP-pinning [`HttpTransport`] with an explicit
    /// [`EgressGuard`]. The transport's resolver is built [`from_guard`](HttpTransport::from_guard),
    /// so the guard's allowlist governs BOTH the pre-flight [`check_endpoint`](EgressGuard::check_endpoint)
    /// and the socket resolver — one allowlist, no second unguarded re-resolve. Native-only.
    /// [OPUS-4.8] sq-25xk.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_guard_native(endpoint: impl Into<String>, guard: EgressGuard) -> Self {
        let transport = Box::new(HttpTransport::from_guard(&guard));
        Endpoint {
            endpoint: endpoint.into(),
            transport,
            guard,
        }
    }

    /// The endpoint URL.
    pub fn url(&self) -> &str {
        &self.endpoint
    }

    /// The egress guard governing this endpoint.
    pub fn guard(&self) -> &EgressGuard {
        &self.guard
    }
}

impl FederatedSource for Endpoint {
    fn source_type(&self) -> SourceType<'_> {
        SourceType::Endpoint(self)
    }

    fn discover(&self) -> Result<(Capability, Option<sparq_fedplan::SourceDescriptor>), FedError> {
        // Phase-2 STUB: the static "full endpoint" capability, no descriptor. Real VoID/SD
        // discovery is Phase 1 (the `discovery` module). HONEST: this does NOT contact the
        // endpoint — it returns the conservative "everything an endpoint can do" default.
        Ok((Capability::endpoint(), None))
    }

    fn execute(&self, sub: &SubQuery) -> Result<String, FedError> {
        // SSRF gate FIRST — refuse a private/internal endpoint before any request. This is
        // the load-bearing safety step: default-deny, allowlist-to-reopen, DNS-rebinding
        // -safe (vets the resolved IP).
        let _host = self.guard.check_endpoint(&self.endpoint)?;
        // Forward the (already capability-narrowed, in later phases) SPARQL to the
        // transport seam, exactly like the engine's SERVICE `fetch`. Phase 2 returns the
        // raw SRJ body; the SRJ→solution parse + streaming is Phase 5.
        self.transport
            .fetch(&self.endpoint, &sub.sparql)
            .map_err(FedError::Transport)
    }
}

// ─── Triple Pattern Fragments: the fragment wire seam + term/pattern model (Phase 6) ─

/// One RDF term in a fragment triple or a query position — the minimal term model the TPF
/// adapters need without re-pulling the engine's full term type. A fragment server speaks
/// concrete RDF, so a triple is three [`FragTerm`]s; a query pattern is three
/// [`PatternTerm`]s (a term, or a named variable). [OPUS-4.8] sq-2qze.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FragTerm {
    /// An IRI (no angle brackets — the bare IRI string).
    Iri(String),
    /// A blank node (the bare label, no `_:`).
    Blank(String),
    /// A literal, stored in its canonical N-Triples lexical form *including* any
    /// `"..."^^<dt>` / `"..."@lang` decoration, so equality is exact and lossless.
    Literal(String),
}

impl FragTerm {
    /// An IRI term from a bare IRI string.
    pub fn iri(s: impl Into<String>) -> FragTerm {
        FragTerm::Iri(s.into())
    }
}

/// One position of a TPF triple-pattern request: a bound [`FragTerm`] or a query variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternTerm {
    /// A bound term — constrains the fragment.
    Bound(FragTerm),
    /// A variable (the bare name, no `?`) — a wildcard the fragment binds.
    Var(String),
}

impl PatternTerm {
    /// The variable name if this position is a variable.
    pub fn as_var(&self) -> Option<&str> {
        match self {
            PatternTerm::Var(v) => Some(v),
            PatternTerm::Bound(_) => None,
        }
    }
}

/// A single triple pattern the planner pushes to a fragment source: subject, predicate,
/// object, each either bound or a variable (exactly the TPF/brTPF access unit — one triple
/// pattern). [OPUS-4.8] sq-2qze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragPattern {
    /// The subject position.
    pub subject: PatternTerm,
    /// The predicate position.
    pub predicate: PatternTerm,
    /// The object position.
    pub object: PatternTerm,
}

impl FragPattern {
    /// A new pattern from its three positions.
    pub fn new(subject: PatternTerm, predicate: PatternTerm, object: PatternTerm) -> FragPattern {
        FragPattern {
            subject,
            predicate,
            object,
        }
    }

    /// The variable names this pattern projects, subject→predicate→object order, skipping
    /// repeats (so a `?s ?p ?s` pattern yields `["s", "p"]`).
    pub fn vars(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(3);
        for pos in [&self.subject, &self.predicate, &self.object] {
            if let Some(v) = pos.as_var() {
                if !out.iter().any(|x: &String| x == v) {
                    out.push(v.to_string());
                }
            }
        }
        out
    }
}

/// A solution mapping over fragment variables: variable name → bound [`FragTerm`]. The unit
/// the planner's join operators consume (the TPF analogue of an SRJ result row).
pub type FragBinding = Vec<(String, FragTerm)>;

/// One concrete RDF triple returned in a fragment's data graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragTriple {
    /// The subject term.
    pub subject: FragTerm,
    /// The predicate term.
    pub predicate: FragTerm,
    /// The object term.
    pub object: FragTerm,
}

impl FragTriple {
    /// A new triple.
    pub fn new(subject: FragTerm, predicate: FragTerm, object: FragTerm) -> FragTriple {
        FragTriple {
            subject,
            predicate,
            object,
        }
    }
}

/// One fragment page: the matched triples on this page, the fragment's estimated total
/// match count (the TPF cardinality oracle — `hydra:totalItems`/`void:triples`, the SAME
/// for every page of a fragment), and an optional opaque next-page token (`hydra:next`).
///
/// The token is whatever the transport needs to fetch the next page (a URL, a page number);
/// `None` means this is the last page. The adapters follow it to exhaustion so the result
/// is COMPLETE. [OPUS-4.8] sq-2qze.
#[derive(Debug, Clone)]
pub struct FragmentPage {
    /// The triples on this page (already filtered to the requested pattern by the server).
    pub triples: Vec<FragTriple>,
    /// `hydra:totalItems` — the fragment's estimated total match count (the cardinality
    /// oracle the planner keys on). Reported per-page but constant across a fragment's pages.
    pub total_items: u64,
    /// The opaque next-page token, or `None` on the last page.
    pub next: Option<String>,
}

/// The TPF/brTPF wire seam: fetch one fragment page for `pattern` from `url`.
///
/// This is the fragment-server analogue of the SRJ [`Transport`] seam. One method covers
/// both interfaces: a plain-TPF request passes `bindings = &[]`; a brTPF request passes a
/// block of *at most `maxMpR`* solution mappings the server uses to pre-filter the fragment
/// (only triples joining at least one binding come back). `page` is the next-page token
/// from a prior [`FragmentPage::next`] (`None` for the first page). The implementation
/// returns the page's triples + count + next token, or a transport-error string.
///
/// The native HTTP implementation is [`HttpFragmentTransport`] (bead `sq-yzca`): ureq + the
/// default-deny SSRF resolver, serialising the pattern into the Hydra URI template + the binding
/// block as the brTPF `values` parameter, and parsing the Turtle/TriG fragment body. The trait
/// is also exercised through an in-memory fixture server in the tests (zero network). [OPUS-4.8].
pub trait FragmentTransport: Send + Sync {
    /// Fetch one fragment page. `bindings` is the brTPF block (empty for plain TPF);
    /// `page` is the next-page token (`None` for the first page).
    fn fetch_fragment(
        &self,
        url: &str,
        pattern: &FragPattern,
        bindings: &[FragBinding],
        page: Option<&str>,
    ) -> Result<FragmentPage, String>;
}

// ─── Native HTTP FragmentTransport (ureq + SSRF resolver + Hydra template + Turtle/TriG) ─

/// The Hydra Core vocabulary namespace (the TPF control vocabulary). [OPUS-4.8] sq-yzca.
#[cfg(not(target_arch = "wasm32"))]
const HYDRA_NS: &str = "http://www.w3.org/ns/hydra/core#";
/// The VoID namespace (`void:triples` — the fragment's match-count estimate). [OPUS-4.8].
#[cfg(not(target_arch = "wasm32"))]
const VOID_NS: &str = "http://rdfs.org/ns/void#";
/// The brTPF `values` control property the sparq server mints (sq-dxhb) — the
/// `hydra:property` its `{values}` template variable maps to. The native transport attaches
/// the brTPF binding block under the `values` query parameter (the server's text wire).
/// [OPUS-4.8] sq-yzca.
#[cfg(not(target_arch = "wasm32"))]
const BRTPF_VALUES_PARAM: &str = "values";

/// The production [`FragmentTransport`]: a blocking ureq GET against a Triple-Pattern-Fragments
/// (TPF) / bindings-restricted-TPF (brTPF) server, behind the **same default-deny SSRF resolver**
/// as [`HttpTransport`] (the resolved-and-vetted IP is the dialled IP — no DNS-rebinding
/// re-resolve window).
///
/// # What it does (the four pieces the bead names)
///
/// 1. **Hydra URI-template serialisation.** A [`FragPattern`] is rendered into the TPF query
///    string the sparq server reads (`?subject=&predicate=&object=`, each a percent-encoded
///    N-Triples term; a variable position is omitted). This is the `{subject}`/`{predicate}`/
///    `{object}` Hydra template the server advertises (`sparq-server`'s `tpf.rs`).
/// 2. **brTPF binding block.** A non-empty `bindings` slice is attached as the `values` query
///    parameter using the server's text wire ([`crate::wire::encode_bindings_text`]), so the
///    server returns only triples joining at least one attached mapping (the brTPF bind-join).
/// 3. **Pagination.** A `page` token is the OPAQUE next-page URL the server published as
///    `hydra:next`; the transport GETs it verbatim, so following pagination to exhaustion is
///    "GET the `hydra:next` link until there is none" — exactly what `drain_fragment` drives.
/// 4. **Turtle/TriG fragment-body parse.** The response (`Accept: text/turtle, application/trig`)
///    is parsed with oxttl's TriG parser (a superset of Turtle — it handles a default-graph
///    Turtle body and a named-graph TriG body identically). The parse SPLITS the document into
///    the **control** triples (`hydra:totalItems` / `void:triples` → the count; `hydra:next` →
///    the next-page link) and the **data** triples (everything whose predicate is NOT in the
///    Hydra/VoID control vocabulary), and the data triples are filtered to those that actually
///    match the requested pattern, so a control triple can never be mistaken for a match.
///
/// Native-only (`cfg(not(target_arch = "wasm32"))`): ureq + its TLS stack never enter the wasm
/// bundle (a wasm federation client would carry a `fetch`-based transport). Construct with
/// [`HttpFragmentTransport::new`] (strict default-deny) or [`HttpFragmentTransport::from_guard`]
/// to share an [`EgressGuard`]'s allowlist. [OPUS-4.8] sq-yzca.
#[cfg(not(target_arch = "wasm32"))]
pub struct HttpFragmentTransport {
    timeout: std::time::Duration,
    /// Hosts (bare authority, lowercased) the SSRF resolver permits even when they resolve
    /// privately — bridged from an [`EgressGuard`] so the pre-flight check and the socket
    /// resolver share one allowlist.
    allow_private: std::sync::Arc<HashSet<String>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for HttpFragmentTransport {
    fn default() -> Self {
        HttpFragmentTransport::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl HttpFragmentTransport {
    /// A fragment transport with a finite default timeout and the strict default-deny SSRF
    /// policy (no private host permitted).
    pub fn new() -> HttpFragmentTransport {
        HttpFragmentTransport {
            timeout: std::time::Duration::from_secs(30),
            allow_private: std::sync::Arc::new(HashSet::new()),
        }
    }

    /// A fragment transport whose SSRF resolver shares `guard`'s allowlist, so a deliberately
    /// allowlisted private fragment server is reachable (and only such a host is). The IP the
    /// resolver vets is the IP ureq dials.
    pub fn from_guard(guard: &EgressGuard) -> HttpFragmentTransport {
        HttpFragmentTransport {
            timeout: std::time::Duration::from_secs(30),
            allow_private: std::sync::Arc::new(guard.allowed_hosts().clone()),
        }
    }

    /// Sets the per-request timeout. Chainable.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> HttpFragmentTransport {
        self.timeout = timeout;
        self
    }

    /// Build the first-page request URL for `url` + `pattern` (+ optional brTPF `bindings`):
    /// the Hydra TPF query string with the bound positions percent-encoded as N-Triples terms.
    fn first_page_url(url: &str, pattern: &FragPattern, bindings: &[FragBinding]) -> String {
        let mut out = String::with_capacity(url.len() + 64);
        out.push_str(url);
        let mut sep = if url.contains('?') { '&' } else { '?' };
        for (key, pos) in [
            ("subject", &pattern.subject),
            ("predicate", &pattern.predicate),
            ("object", &pattern.object),
        ] {
            if let PatternTerm::Bound(term) = pos {
                out.push(sep);
                out.push_str(key);
                out.push('=');
                out.push_str(&pct_encode(&term_to_ntriples(term)));
                sep = '&';
            }
        }
        // brTPF binding block (the server's text wire). Empty → plain TPF (no `values` param).
        if !bindings.is_empty() {
            let wire = crate::wire::encode_bindings_text(bindings);
            if !wire.is_empty() {
                out.push(sep);
                out.push_str(BRTPF_VALUES_PARAM);
                out.push('=');
                out.push_str(&pct_encode(&wire));
            }
        }
        out
    }
}

/// Percent-encode a query-parameter VALUE (RFC 3986 unreserved set passes through; everything
/// else is `%XX`-escaped, uppercase hex per UTF-8 byte), so an N-Triples term (`<`, `>`, `"`,
/// spaces, newlines in a brTPF block) round-trips through the URL back to the same term. The
/// server's TPF `hydra:next` builder uses the SAME `percent-encoding` set, so the link the
/// transport builds is read identically server-side; the discovery ASK probe shares this one
/// definition too (#3712).
pub(crate) fn pct_encode(value: &str) -> String {
    const UNRESERVED: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(value, UNRESERVED).to_string()
}

/// Render a [`FragTerm`] in N-Triples lexical form (the TPF query-parameter grammar the server
/// reads): an IRI is `<…>`-wrapped, a blank node is `_:…`, a literal carries its
/// `"…"`/`@lang`/`^^<dt>` decoration verbatim (the `FragTerm::Literal` model already stores it).
/// Identical to `crate::wire`'s private renderer; inlined here so the transport has no cross-
/// module private dependency. [OPUS-4.8] sq-yzca.
#[cfg(not(target_arch = "wasm32"))]
fn term_to_ntriples(term: &FragTerm) -> String {
    match term {
        FragTerm::Iri(s) => format!("<{s}>"),
        FragTerm::Blank(s) => format!("_:{s}"),
        FragTerm::Literal(s) => s.clone(),
    }
}

/// Convert one parsed oxrdf [`oxrdf::Term`] (a triple object) into a [`FragTerm`], preserving the
/// term's canonical N-Triples lexical identity (so equality against a bound pattern position is
/// exact). [OPUS-4.8] sq-yzca.
#[cfg(not(target_arch = "wasm32"))]
fn oxterm_to_fragterm(term: &oxrdf::Term) -> FragTerm {
    match term {
        oxrdf::Term::NamedNode(n) => FragTerm::Iri(n.as_str().to_string()),
        oxrdf::Term::BlankNode(b) => FragTerm::Blank(b.as_str().to_string()),
        // A literal's canonical N-Triples form (`"v"`, `"v"@lang`, `"v"^^<dt>`) is its lexical
        // identity in the `FragTerm::Literal` model — keep it verbatim via `Display`.
        oxrdf::Term::Literal(l) => FragTerm::Literal(l.to_string()),
        // SPARQL-1.2 triple terms cannot appear as a TPF fragment data object; carry the
        // canonical serialisation so a (degenerate) match still compares exactly.
        other => FragTerm::Literal(other.to_string()),
    }
}

/// Convert a parsed oxrdf subject ([`oxrdf::NamedOrBlankNode`]) into a [`FragTerm`]. [OPUS-4.8].
#[cfg(not(target_arch = "wasm32"))]
fn oxsubject_to_fragterm(s: &oxrdf::NamedOrBlankNode) -> FragTerm {
    match s {
        oxrdf::NamedOrBlankNode::NamedNode(n) => FragTerm::Iri(n.as_str().to_string()),
        oxrdf::NamedOrBlankNode::BlankNode(b) => FragTerm::Blank(b.as_str().to_string()),
    }
}

/// Whether a predicate IRI is a TPF **control** predicate (Hydra / VoID), i.e. metadata about
/// the fragment rather than a data triple. The native parser uses this to SPLIT the fragment
/// document: control triples feed the count + next-page link; everything else is candidate
/// data. [OPUS-4.8] sq-yzca.
#[cfg(not(target_arch = "wasm32"))]
fn is_control_predicate(pred: &str) -> bool {
    pred.starts_with(HYDRA_NS)
        || pred.starts_with(VOID_NS)
        // rdf:type triples describe the fragment/dataset/control nodes, never the data.
        || pred == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
}

#[cfg(not(target_arch = "wasm32"))]
impl FragmentTransport for HttpFragmentTransport {
    fn fetch_fragment(
        &self,
        url: &str,
        pattern: &FragPattern,
        bindings: &[FragBinding],
        page: Option<&str>,
    ) -> Result<FragmentPage, String> {
        // The next-page token is the OPAQUE `hydra:next` URL (built by the server, self-
        // contained); the first page is built from the pattern + binding block.
        let request_url = match page {
            Some(next) => next.to_string(),
            None => Self::first_page_url(url, pattern, bindings),
        };
        // [OPUS-4.8] sq-g2xs: ureq-3 `Agent::with_parts` with the default-deny SSRF resolver —
        // ureq dials only the addresses the resolver returns, so the vetted IP IS the dialled IP
        // (no DNS-rebinding re-resolve window). Same resolver as the SRJ `HttpTransport`.
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .user_agent(concat!("sparq-fedclient/", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::new(),
            EgressFilterResolver {
                allow_private: std::sync::Arc::clone(&self.allow_private),
            },
        );
        let body = match agent
            .get(&request_url)
            // TriG is a Turtle superset; ask for either so a server that serves Turtle (the
            // conventional TPF serialisation) or TriG (named graphs) both parse.
            .header("Accept", "application/trig, text/turtle")
            .call()
        {
            Ok(mut r) => r
                .body_mut()
                .with_config()
                .limit(MAX_BODY_BYTES)
                .read_to_string()
                .map_err(|e| format!("fedclient: reading fragment from {request_url}: {e}"))?,
            Err(ureq::Error::StatusCode(code)) => {
                return Err(format!(
                    "fedclient: fragment server {request_url} returned HTTP {code}"
                ))
            }
            Err(e) => return Err(format!("fedclient: request to {request_url} failed: {e}")),
        };
        parse_fragment_body(&body, pattern)
    }
}

/// Parse a Turtle/TriG fragment body into a [`FragmentPage`] (the data triples + the
/// `hydra:totalItems`/`void:triples` count + the `hydra:next` token).
///
/// TriG is parsed (a superset of Turtle: a Turtle body is a TriG document with everything in the
/// default graph), so a plain-TPF Turtle response and a TriG response are handled identically.
/// The parse SPLITS each parsed triple into:
///
/// * a **control** triple (predicate in the Hydra/VoID vocabulary or `rdf:type`) — from which the
///   match-count (`hydra:totalItems` / `void:triples`, the max seen) and the next-page link
///   (`hydra:next`, an IRI object) are read; and
/// * a candidate **data** triple — kept iff it actually MATCHES the requested `pattern` (the same
///   consistency [`bind_triple`] enforces downstream), so a control triple never leaks in as a
///   match and a misbehaving server cannot smuggle a non-matching triple through.
///
/// A malformed body (a TriG syntax error) is a clean transport-error string, never a panic — this
/// crate is `forbid(unsafe_code)`. [OPUS-4.8] sq-yzca.
#[cfg(not(target_arch = "wasm32"))]
fn parse_fragment_body(body: &str, pattern: &FragPattern) -> Result<FragmentPage, String> {
    let mut triples: Vec<FragTriple> = Vec::new();
    let mut total_items: u64 = 0;
    let mut next: Option<String> = None;
    for quad in oxttl::TriGParser::new().for_slice(body.as_bytes()) {
        let quad = quad.map_err(|e| format!("fedclient: malformed TriG/Turtle fragment: {e}"))?;
        let pred = quad.predicate.as_str();
        if is_control_predicate(pred) {
            // Count metadata: take the largest of any `hydra:totalItems` / `void:triples` literal
            // (a fragment may carry the count on more than one control node; they agree, and the
            // max is the recall-safe choice if they ever differ).
            if pred == format!("{HYDRA_NS}totalItems") || pred == format!("{VOID_NS}triples") {
                if let oxrdf::Term::Literal(l) = &quad.object {
                    if let Ok(n) = l.value().parse::<u64>() {
                        total_items = total_items.max(n);
                    }
                }
            }
            // The next-page control link (`hydra:next`; legacy `hydra:nextPage` also accepted).
            if pred == format!("{HYDRA_NS}next") || pred == format!("{HYDRA_NS}nextPage") {
                if let oxrdf::Term::NamedNode(n) = &quad.object {
                    next = Some(n.as_str().to_string());
                }
            }
            continue;
        }
        // A candidate data triple: keep it only if it matches the requested pattern (the
        // load-bearing answer-safety filter — a control triple or a non-matching triple cannot
        // become a match).
        let triple = FragTriple::new(
            oxsubject_to_fragterm(&quad.subject),
            FragTerm::Iri(pred.to_string()),
            oxterm_to_fragterm(&quad.object),
        );
        if bind_triple(pattern, &triple).is_some() {
            triples.push(triple);
        }
    }
    Ok(FragmentPage {
        triples,
        total_items,
        next,
    })
}

// ─── Bind one matched triple back into a pattern's variables ─────────────────────────

/// Bind a concrete `triple` (returned by the fragment server for `pattern`) into the
/// pattern's variables, returning `None` if the triple is inconsistent with the pattern
/// (a bound position disagrees, or the same variable in two positions binds two different
/// terms — so a self-join pattern like `?s ?p ?s` only yields a row when the positions
/// agree). This is the load-bearing correctness step: a misbehaving fragment server that
/// returns a non-matching triple cannot smuggle a wrong binding through. [OPUS-4.8] sq-2qze.
fn bind_triple(pattern: &FragPattern, triple: &FragTriple) -> Option<FragBinding> {
    let mut out: FragBinding = Vec::with_capacity(3);
    for (pos, term) in [
        (&pattern.subject, &triple.subject),
        (&pattern.predicate, &triple.predicate),
        (&pattern.object, &triple.object),
    ] {
        match pos {
            PatternTerm::Bound(b) => {
                if b != term {
                    return None; // server returned a triple that does not match a bound slot
                }
            }
            PatternTerm::Var(v) => {
                // If this variable already bound (repeated var), the terms must agree.
                if let Some((_, prev)) = out.iter().find(|(name, _)| name == v) {
                    if prev != term {
                        return None;
                    }
                } else {
                    out.push((v.clone(), term.clone()));
                }
            }
        }
    }
    Some(out)
}

/// Split `bindings` into blocks of at most `max_mpr` (≥1) mappings — the brTPF
/// `maxMpR`-bounded binding blocks. A `max_mpr` of 0 is treated as 1 (a degenerate cap must
/// not produce zero-sized blocks, which would loop forever). [OPUS-4.8] sq-2qze.
fn chunk_bindings(bindings: &[FragBinding], max_mpr: u32) -> Vec<&[FragBinding]> {
    let block = max_mpr.max(1) as usize;
    if bindings.is_empty() {
        return Vec::new();
    }
    bindings.chunks(block).collect()
}

/// Drive a fragment fetch to exhaustion (follow `hydra:next` until `None`), binding every
/// returned triple into `pattern` and collecting the consistent rows. Returns the bound
/// rows plus the fragment's `hydra:totalItems` (from the first page; constant per fragment).
/// `page_cap` bounds the number of pages followed so a buggy server that never stops
/// paginating cannot hang the client (a defensive bound, not a correctness limit — a
/// well-behaved server terminates with `next = None` well within it). [OPUS-4.8] sq-2qze.
fn drain_fragment(
    transport: &dyn FragmentTransport,
    url: &str,
    pattern: &FragPattern,
    bindings: &[FragBinding],
    page_cap: usize,
) -> Result<(Vec<FragBinding>, u64), FedError> {
    let mut rows: Vec<FragBinding> = Vec::new();
    let mut next: Option<String> = None;
    let mut total_items = 0u64;
    let mut first = true;
    for _ in 0..page_cap {
        let page = transport
            .fetch_fragment(url, pattern, bindings, next.as_deref())
            .map_err(FedError::Transport)?;
        if first {
            total_items = page.total_items;
            first = false;
        }
        for t in &page.triples {
            if let Some(b) = bind_triple(pattern, t) {
                rows.push(b);
            }
        }
        match page.next {
            Some(tok) => next = Some(tok),
            None => return Ok((rows, total_items)),
        }
    }
    Err(FedError::Transport(format!(
        "fragment pagination exceeded the {} page safety cap for {:?} (next-page link never \
         terminated)",
        page_cap, url
    )))
}

/// The defensive page cap (see [`drain_fragment`]): follows up to this many `hydra:next`
/// links before refusing. Generous enough for any real fragment, finite enough to fail-stop
/// a runaway server.
const FRAGMENT_PAGE_CAP: usize = 1_000_000;

/// Build a one-pattern [`SourceDescriptor`] from a fragment's count metadata so the planner's
/// CostFed estimate keys on the *served* `hydra:totalItems` for this source. When the pattern
/// has a bound predicate IRI the count is attributed to that predicate partition (the exact
/// signal `estimate_cardinality` reads); otherwise it seeds the descriptor's total only.
/// [OPUS-4.8] sq-2qze.
fn descriptor_from_count(
    id: &str,
    pattern: &FragPattern,
    total_items: u64,
) -> sparq_fedplan::SourceDescriptor {
    use sparq_fedplan::{SourceDescriptor, SourceId};
    let mut builder = SourceDescriptor::builder(SourceId::new(id)).total_triples(total_items);
    if let PatternTerm::Bound(FragTerm::Iri(pred)) = &pattern.predicate {
        builder = builder.predicate(sparq_fedplan::PredPartition {
            predicate: pred.clone(),
            triples: total_items,
            distinct_subjects: 0,
            distinct_objects: 0,
        });
    }
    builder.build()
}

// ─── brTPF / TPF adapters (Phase 6 — real) + Local adapter (planner phases) ──────────

/// A bindings-restricted Triple Pattern Fragments source (brTPF). **Phase 6 — real.**
///
/// Wraps a [`FragmentTransport`] and a `maxMpR` bind-join bound. [`solutions`](Self::solutions)
/// answers one triple pattern against a block of upstream `bindings`: it chunks the bindings
/// into `maxMpR`-sized blocks, issues one paginated fragment request per block (the brTPF
/// bind-join — the server returns only triples joining at least one attached binding), binds
/// every returned triple back into the pattern, and concatenates the per-block matches. The
/// result is **complete**: every upstream binding sits in exactly one block, and every
/// matching triple for that block comes back, so no join result is lost. With an empty
/// `bindings` slice brTPF degrades to a plain fragment scan (one unbound request).
///
/// `discover()` reports the brTPF [`Capability`] and a count-metadata [`SourceDescriptor`](sparq_fedplan::SourceDescriptor)
/// for the open pattern (`?s ?p ?o`), the recall-safe upper-bound cardinality the planner
/// keys on. [OPUS-4.8] sq-2qze.
pub struct BrTpfSource {
    /// The brTPF fragment-template / base URL.
    pub url: String,
    /// `maxMpR` — at most this many bound mappings per request.
    pub max_mpr: u32,
    transport: Box<dyn FragmentTransport>,
}

impl BrTpfSource {
    /// A new brTPF source over `transport` with the `max_mpr` bind-join bound.
    pub fn new(
        url: impl Into<String>,
        max_mpr: u32,
        transport: Box<dyn FragmentTransport>,
    ) -> Self {
        BrTpfSource {
            url: url.into(),
            max_mpr,
            transport,
        }
    }

    /// Answer `pattern` against this brTPF source, pushing `bindings` in `maxMpR`-bounded
    /// blocks (the brTPF bind-join). Returns the complete set of bound solution mappings.
    /// An empty `bindings` slice issues a single unbound fragment scan.
    pub fn solutions(
        &self,
        pattern: &FragPattern,
        bindings: &[FragBinding],
    ) -> Result<Vec<FragBinding>, FedError> {
        let blocks = chunk_bindings(bindings, self.max_mpr);
        if blocks.is_empty() {
            // No upstream bindings → a plain unbound fragment scan (still complete).
            let (rows, _) = drain_fragment(
                self.transport.as_ref(),
                &self.url,
                pattern,
                &[],
                FRAGMENT_PAGE_CAP,
            )?;
            return Ok(rows);
        }
        let mut all: Vec<FragBinding> = Vec::new();
        for block in blocks {
            let (rows, _) = drain_fragment(
                self.transport.as_ref(),
                &self.url,
                pattern,
                block,
                FRAGMENT_PAGE_CAP,
            )?;
            all.extend(rows);
        }
        Ok(all)
    }

    /// The fragment's count metadata for the open pattern (`hydra:totalItems`) — the TPF
    /// cardinality oracle. A network round-trip (one unbound first page).
    pub fn cardinality(&self, pattern: &FragPattern) -> Result<u64, FedError> {
        let page = self
            .transport
            .fetch_fragment(&self.url, pattern, &[], None)
            .map_err(FedError::Transport)?;
        Ok(page.total_items)
    }
}

impl FederatedSource for BrTpfSource {
    fn source_type(&self) -> SourceType<'_> {
        SourceType::BrTpf(self)
    }
    fn discover(&self) -> Result<(Capability, Option<sparq_fedplan::SourceDescriptor>), FedError> {
        // Count-metadata descriptor for the OPEN pattern (?s ?p ?o): the recall-safe
        // upper-bound cardinality. The bound block only ever NARROWS this at execution.
        let open = FragPattern::new(
            PatternTerm::Var("s".into()),
            PatternTerm::Var("p".into()),
            PatternTerm::Var("o".into()),
        );
        let total = self.cardinality(&open)?;
        let desc = descriptor_from_count(&self.url, &open, total);
        Ok((Capability::brtpf(self.max_mpr), Some(desc)))
    }
    fn execute(&self, _sub: &SubQuery) -> Result<String, FedError> {
        // A fragment server speaks triples, not SPARQL-Results-JSON. The brTPF answer is the
        // typed solution set — call `solutions(pattern, bindings)`. We do NOT lossily
        // re-serialise through SRJ here (no overclaim).
        Err(FedError::Unsupported(
            "brTPF source answers triple-pattern fragments — call `BrTpfSource::solutions` \
             (pattern, bindings); a fragment server returns triples, not SPARQL-Results-JSON"
                .to_string(),
        ))
    }
}

/// A plain Triple Pattern Fragments source (TPF). **Phase 6 — real.**
///
/// Wraps a [`FragmentTransport`]. [`solutions`](Self::solutions) answers one triple pattern
/// by fetching its fragment to exhaustion (following `hydra:next`) and binding every matched
/// triple into the pattern's variables. There is **no** bind-join: a plain-TPF source shifts
/// every join client-side, so the adapter materialises the whole (selective) fragment for
/// the planner to greedy count-driven hash-join locally (design §2.1). `discover()` reports
/// the TPF [`Capability`] and the count-metadata [`SourceDescriptor`](sparq_fedplan::SourceDescriptor). [OPUS-4.8] sq-2qze.
pub struct TpfSource {
    /// The TPF fragment-template / base URL.
    pub url: String,
    transport: Box<dyn FragmentTransport>,
}

impl TpfSource {
    /// A new plain-TPF source over `transport`.
    pub fn new(url: impl Into<String>, transport: Box<dyn FragmentTransport>) -> Self {
        TpfSource {
            url: url.into(),
            transport,
        }
    }

    /// Answer `pattern` by materialising its fragment to exhaustion and binding every
    /// matched triple. The complete set of bound solution mappings for this pattern.
    pub fn solutions(&self, pattern: &FragPattern) -> Result<Vec<FragBinding>, FedError> {
        let (rows, _) = drain_fragment(
            self.transport.as_ref(),
            &self.url,
            pattern,
            &[],
            FRAGMENT_PAGE_CAP,
        )?;
        Ok(rows)
    }

    /// The fragment's count metadata for `pattern` (`hydra:totalItems`) — the TPF
    /// cardinality oracle that drives the greedy smallest-count-first client-side join order.
    pub fn cardinality(&self, pattern: &FragPattern) -> Result<u64, FedError> {
        let page = self
            .transport
            .fetch_fragment(&self.url, pattern, &[], None)
            .map_err(FedError::Transport)?;
        Ok(page.total_items)
    }
}

impl FederatedSource for TpfSource {
    fn source_type(&self) -> SourceType<'_> {
        SourceType::Tpf(self)
    }
    fn discover(&self) -> Result<(Capability, Option<sparq_fedplan::SourceDescriptor>), FedError> {
        let open = FragPattern::new(
            PatternTerm::Var("s".into()),
            PatternTerm::Var("p".into()),
            PatternTerm::Var("o".into()),
        );
        let total = self.cardinality(&open)?;
        let desc = descriptor_from_count(&self.url, &open, total);
        Ok((Capability::tpf(), Some(desc)))
    }
    fn execute(&self, _sub: &SubQuery) -> Result<String, FedError> {
        Err(FedError::Unsupported(
            "TPF source answers triple-pattern fragments — call `TpfSource::solutions(pattern)`; \
             a fragment server returns triples, not SPARQL-Results-JSON"
                .to_string(),
        ))
    }
}

/// The in-process local engine source over a `Graph`. The capability is "everything"; the
/// actual local BGP evaluation through `sparq-engine` is wired in the planner/operators
/// phases, so `execute` is a clearly-labelled stub here (no overclaim). The source id lets
/// the planner address it. [OPUS-4.8] sq-rsxf.
pub struct LocalSource {
    /// A stable id for the local source (used by the planner to address it).
    pub id: String,
}

impl FederatedSource for LocalSource {
    fn source_type(&self) -> SourceType<'_> {
        SourceType::Local(self)
    }
    fn discover(&self) -> Result<(Capability, Option<sparq_fedplan::SourceDescriptor>), FedError> {
        Ok((Capability::local(), None))
    }
    fn execute(&self, _sub: &SubQuery) -> Result<String, FedError> {
        Err(FedError::Unsupported(
            "local-engine source execution is wired in the planner/operators phases (Phase 3/5)"
                .to_string(),
        ))
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::Mutex;

    /// #3712 tripwire: every printable ASCII byte + a control + 2-/4-byte UTF-8 — exactly the
    /// RFC 3986 unreserved set passes through; the rest is uppercase `%XX` per UTF-8 byte.
    #[test]
    fn pct_encode_pins_the_unreserved_set() {
        let all: String = (0x20u8..0x7f)
            .map(char::from)
            .chain("\t\né😀".chars())
            .collect();
        assert_eq!(
            pct_encode(&all),
            "%20%21%22%23%24%25%26%27%28%29%2A%2B%2C-.%2F0123456789%3A%3B%3C%3D%3E%3F%40ABCDEFGHIJKLMNOPQRSTUVWXYZ%5B%5C%5D%5E_%60abcdefghijklmnopqrstuvwxyz%7B%7C%7D~%09%0A%C3%A9%F0%9F%98%80"
        );
    }

    /// A canned-response transport double: records the (endpoint, query) it was asked for
    /// and returns a fixed body. Lets us assert the adapter reached the transport (i.e.
    /// the SSRF gate ALLOWED) without any network. [OPUS-4.8] sq-rsxf.
    struct CannedTransport {
        body: String,
        seen: Mutex<Vec<(String, String)>>,
    }
    impl CannedTransport {
        fn new(body: &str) -> Self {
            CannedTransport {
                body: body.to_string(),
                seen: Mutex::new(Vec::new()),
            }
        }
    }
    impl Transport for CannedTransport {
        fn fetch(&self, endpoint: &str, query: &str) -> Result<String, String> {
            self.seen
                .lock()
                .unwrap()
                .push((endpoint.to_string(), query.to_string()));
            Ok(self.body.clone())
        }
    }

    /// A transport that PANICS if called — proves the SSRF gate refused before any
    /// transport round-trip (a deny must never reach `fetch`). [OPUS-4.8] sq-rsxf.
    struct PanicTransport;
    impl Transport for PanicTransport {
        fn fetch(&self, _endpoint: &str, _query: &str) -> Result<String, String> {
            panic!("transport must NOT be reached when the SSRF gate denies the endpoint");
        }
    }

    // ── is_forbidden_ip classification (mirrors the engine's policy) ──────────────

    #[test]
    fn forbidden_ips_are_refused() {
        for ip in [
            "127.0.0.1",       // loopback
            "10.0.0.5",        // RFC1918
            "172.16.0.1",      // RFC1918
            "192.168.1.1",     // RFC1918
            "169.254.169.254", // cloud metadata (link-local)
            "0.0.0.0",         // unspecified
            "255.255.255.255", // broadcast
            "100.64.0.1",      // CGNAT
        ] {
            assert!(
                is_forbidden_ip(ip.parse().unwrap()),
                "{ip} should be forbidden"
            );
        }
        // IPv6
        assert!(is_forbidden_ip(IpAddr::V6(Ipv6Addr::LOCALHOST))); // ::1
        assert!(is_forbidden_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED))); // ::
        assert!(is_forbidden_ip("fe80::1".parse().unwrap())); // link-local
        assert!(is_forbidden_ip("fc00::1".parse().unwrap())); // unique-local
                                                              // IPv4-mapped private v4 must not smuggle through a v6 literal.
        assert!(is_forbidden_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_forbidden_ip("::ffff:10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn public_ips_are_allowed() {
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34"] {
            assert!(
                !is_forbidden_ip(ip.parse().unwrap()),
                "{ip} should be allowed"
            );
        }
        assert!(!is_forbidden_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5))));
        assert!(!is_forbidden_ip("2606:4700:4700::1111".parse().unwrap())); // public v6
    }

    // ── endpoint_host parsing ─────────────────────────────────────────────────────

    #[test]
    fn host_parsing() {
        assert_eq!(
            endpoint_host("http://dbpedia.org/sparql").as_deref(),
            Some("dbpedia.org")
        );
        assert_eq!(
            endpoint_host("https://EXAMPLE.org:8443/q").as_deref(),
            Some("example.org")
        );
        assert_eq!(
            endpoint_host("http://user:pw@host.example/p").as_deref(),
            Some("host.example")
        );
        assert_eq!(
            endpoint_host("http://[::1]:8080/sparql").as_deref(),
            Some("::1")
        );
        assert_eq!(
            endpoint_host("http://127.0.0.1/sparql").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(endpoint_host("not-a-url"), None);
        assert_eq!(endpoint_host("http:///nohost"), None);
    }

    #[test]
    fn host_port_parsing() {
        // Direct unit test for the port-aware authority parser (coverage ratchet, sq-vbnyc).
        // Explicit port preserved.
        assert_eq!(
            endpoint_host_port("http://127.0.0.1:8053/sparql"),
            Some(("127.0.0.1".to_string(), 8053))
        );
        assert_eq!(
            endpoint_host_port("https://EXAMPLE.org:8443/q"),
            Some(("example.org".to_string(), 8443))
        );
        assert_eq!(
            endpoint_host_port("http://[2001:db8::1]:8080/sparql"),
            Some(("2001:db8::1".to_string(), 8080))
        );
        // Scheme defaults when no explicit port (443 for https, 80 otherwise).
        assert_eq!(
            endpoint_host_port("http://dbpedia.org/sparql"),
            Some(("dbpedia.org".to_string(), 80))
        );
        assert_eq!(
            endpoint_host_port("https://dbpedia.org/sparql"),
            Some(("dbpedia.org".to_string(), 443))
        );
        assert_eq!(
            endpoint_host_port("http://[::1]/sparql"),
            Some(("::1".to_string(), 80))
        );
        // Userinfo stripped; no authority → None.
        assert_eq!(
            endpoint_host_port("http://user:pw@host.example:9000/p"),
            Some(("host.example".to_string(), 9000))
        );
        assert_eq!(endpoint_host_port("http:///nohost"), None);
        // [FABLE-5] sq-3dyje.6: a BARE (unbracketed) multi-colon IPv6 authority must NOT be
        // split on its last colon into host + port — the `!h.contains(':')` guard keeps the
        // whole thing as the host with the scheme default port. Mutating that guard to `true`
        // would wrongly split `fe80::1` into host `fe80:` + port 1. (No earlier test exercised
        // a bare multi-colon authority, so the guard survived.)
        assert_eq!(
            endpoint_host_port("http://fe80::1/sparql"),
            Some(("fe80::1".to_string(), 80)),
            "a bare multi-colon IPv6 authority keeps the whole host + default port, never split"
        );
        assert_eq!(
            endpoint_host_port("https://2001:db8::dead:beef/q"),
            Some(("2001:db8::dead:beef".to_string(), 443))
        );
    }

    // ── DENY: default guard refuses a loopback / private endpoint ─────────────────

    #[test]
    fn deny_loopback_endpoint_literal() {
        let g = EgressGuard::deny_private();
        let err = g
            .check_endpoint("http://127.0.0.1:8080/sparql")
            .unwrap_err();
        assert!(matches!(err, FedError::EgressRefused(_)), "got {err:?}");
    }

    #[test]
    fn deny_ipv6_loopback_endpoint_literal() {
        let g = EgressGuard::deny_private();
        assert!(matches!(
            g.check_endpoint("http://[::1]:8080/sparql").unwrap_err(),
            FedError::EgressRefused(_)
        ));
    }

    #[test]
    fn deny_private_rfc1918_endpoint_literal() {
        let g = EgressGuard::deny_private();
        assert!(matches!(
            g.check_endpoint("http://10.0.0.5/sparql").unwrap_err(),
            FedError::EgressRefused(_)
        ));
    }

    #[test]
    fn deny_cloud_metadata_endpoint_literal() {
        let g = EgressGuard::deny_private();
        assert!(matches!(
            g.check_endpoint("http://169.254.169.254/latest/meta-data/")
                .unwrap_err(),
            FedError::EgressRefused(_)
        ));
    }

    /// The full adapter path: a default-deny endpoint pointing at loopback must refuse
    /// WITHOUT ever calling the transport (PanicTransport would panic if reached).
    #[test]
    fn endpoint_execute_denies_loopback_before_transport() {
        let ep = Endpoint::new("http://127.0.0.1:9999/sparql", Box::new(PanicTransport));
        let err = ep.execute(&SubQuery::new("ASK {}")).unwrap_err();
        assert!(matches!(err, FedError::EgressRefused(_)), "got {err:?}");
    }

    // ── ALLOW: allowlisting a private host re-opens it ────────────────────────────

    #[test]
    fn allow_listed_private_literal_is_permitted() {
        // The literal authority "127.0.0.1" is allowlisted, so check_endpoint returns Ok.
        let g = EgressGuard::deny_private().allow_host("127.0.0.1");
        assert_eq!(
            g.check_endpoint("http://127.0.0.1:8080/sparql").unwrap(),
            "127.0.0.1"
        );
    }

    #[test]
    fn allow_listed_named_host_is_permitted() {
        // A DNS name on the allowlist short-circuits (no lookup), even if it would resolve
        // privately — matching the engine's allowlist-bypasses-resolution behaviour.
        let g = EgressGuard::deny_private().allow_host("sparql.internal");
        assert_eq!(
            g.check_endpoint("http://sparql.internal/sparql").unwrap(),
            "sparql.internal"
        );
    }

    /// The full adapter path under an allowlist: the endpoint resolves privately but the
    /// host is allowlisted, so the transport IS reached and returns the canned body.
    #[test]
    fn endpoint_execute_allows_listed_private_host_and_reaches_transport() {
        let body = r#"{"head":{"vars":[]},"results":{"bindings":[]}}"#;
        let transport = Box::new(CannedTransport::new(body));
        let guard = EgressGuard::deny_private().allow_host("localhost.internal");
        let ep = Endpoint::with_guard("http://localhost.internal:7777/sparql", transport, guard);
        let got = ep
            .execute(&SubQuery::new("SELECT * WHERE { ?s ?p ?o } LIMIT 1"))
            .unwrap();
        assert_eq!(got, body);
    }

    /// A PUBLIC endpoint with the default-deny guard reaches the transport (the guard only
    /// refuses private/internal). We use an IP literal so the test does no real DNS.
    #[test]
    fn endpoint_execute_public_literal_reaches_transport() {
        let body = r#"{"head":{"vars":["s"]},"results":{"bindings":[]}}"#;
        let transport = CannedTransport::new(body);
        // Hold a raw pointer-free reference by constructing then querying `seen` after.
        let ep = Endpoint::new("http://8.8.8.8/sparql", Box::new(transport));
        let got = ep.execute(&SubQuery::new("ASK { ?s ?p ?o }")).unwrap();
        assert_eq!(got, body);
    }

    // ── Port-scoped allowlist entries (sq-vbnyc, mirrors engine sq-a7jw4) ─────────
    //
    // These adversarial cases mirror `sparq-engine`'s SERVICE-egress port-scoping tests on the
    // fedclient guard. The fedclient guard delegates the per-entry decision to the engine's
    // shared `allowlist_entry_permits`, so the two guards MUST agree on every host:port case.

    #[test]
    fn is_allowed_port_exact_host_port_permitted() {
        // (a) A `host:port` entry permits ONLY that host on THAT exact port.
        let g = EgressGuard::deny_private().allow_host("127.0.0.1:8053");
        assert!(g.is_allowed_port("127.0.0.1", 8053)); // exact host:port
    }

    #[test]
    fn is_allowed_port_same_host_other_port_rejected() {
        // (b) The same host on any OTHER port is rejected — strictly narrower than host-level.
        let g = EgressGuard::deny_private().allow_host("127.0.0.1:8053");
        assert!(!g.is_allowed_port("127.0.0.1", 8054)); // other port
        assert!(!g.is_allowed_port("127.0.0.1", 80)); //   default port
    }

    #[test]
    fn is_allowed_port_different_host_rejected() {
        // (c) A different host on the entry's port is rejected.
        let g = EgressGuard::deny_private().allow_host("127.0.0.1:8053");
        assert!(!g.is_allowed_port("127.0.0.2", 8053));
    }

    #[test]
    fn host_level_entry_permits_all_ports_backward_compat() {
        // (d) An existing host-level entry (no port) keeps its meaning: every port on that host.
        // This is the unchanged pre-sq-vbnyc semantics.
        let g = EgressGuard::deny_private().allow_host("127.0.0.1");
        assert!(g.is_allowed_port("127.0.0.1", 80));
        assert!(g.is_allowed_port("127.0.0.1", 8053));
        assert!(g.is_allowed_port("127.0.0.1", 65535));
        assert!(!g.is_allowed_port("127.0.0.2", 80)); // different host still rejected
        // The host-level convenience query stays true for the bare host (any port).
        assert!(g.is_allowed("127.0.0.1"));
    }

    #[test]
    fn port_scoped_suffix_wildcard_is_port_constrained() {
        // A port on a suffix-wildcard entry constrains the port too: `.example.org:443` permits
        // any subdomain (and the apex) on 443 only.
        let g = EgressGuard::deny_private().allow_host(".example.org:443");
        assert!(g.is_allowed_port("sparql.example.org", 443)); // subdomain on 443
        assert!(g.is_allowed_port("example.org", 443)); // apex on 443
        assert!(!g.is_allowed_port("sparql.example.org", 80)); // wrong port
        assert!(!g.is_allowed_port("notexample.org", 443)); // boundary respected
    }

    #[test]
    fn bracketed_ipv6_port_scoped_entry() {
        // A bracketed IPv6 `[::1]:8080` entry is port-scoped on the bare `::1` host.
        let g = EgressGuard::deny_private().allow_host("[::1]:8080");
        assert!(g.is_allowed_port("::1", 8080));
        assert!(!g.is_allowed_port("::1", 80));
    }

    #[test]
    fn bare_ipv6_entry_is_host_level_not_port_amputated() {
        // A bare (unbracketed) IPv6 literal must NOT have its last hextet read as a port —
        // `2001:db8::1` is a host-level entry matching every port (fail-safe, not fail-open).
        let g = EgressGuard::deny_private().allow_host("2001:db8::1");
        assert!(g.is_allowed_port("2001:db8::1", 443));
        assert!(g.is_allowed_port("2001:db8::1", 80));
    }

    #[test]
    fn malformed_port_in_entry_fails_closed() {
        // A `:port` that does not parse as a u16 is treated as part of a never-matching host
        // pattern, NOT as "drop the port constraint and match every port" — fail-CLOSED, never
        // widening the allowlist. (port-0 overflow `:99999`, trailing colon `:`, non-numeric.)
        for entry in ["127.0.0.1:99999", "127.0.0.1:", "127.0.0.1:http"] {
            let g = EgressGuard::deny_private().allow_host(entry);
            assert!(!g.is_allowed_port("127.0.0.1", 80), "{entry} widened port 80");
            assert!(
                !g.is_allowed_port("127.0.0.1", 65535),
                "{entry} widened port 65535"
            );
            assert!(!g.is_allowed("127.0.0.1"), "{entry} widened host-level query");
        }
    }

    #[test]
    fn check_endpoint_honours_port_scoped_entry_end_to_end() {
        // The end-to-end guard path: a private literal allowlisted ONLY on :8053 is permitted on
        // 8053 and refused on 8080 (the scheme default would otherwise apply). This is the load-
        // bearing invariant — the port that is dialled is the port that is vetted.
        let g = EgressGuard::deny_private().allow_host("127.0.0.1:8053");
        assert_eq!(
            g.check_endpoint("http://127.0.0.1:8053/sparql").unwrap(),
            "127.0.0.1"
        );
        assert!(matches!(
            g.check_endpoint("http://127.0.0.1:8080/sparql").unwrap_err(),
            FedError::EgressRefused(_)
        ));
        // The scheme default (80) is also refused — only :8053 is open.
        assert!(matches!(
            g.check_endpoint("http://127.0.0.1/sparql").unwrap_err(),
            FedError::EgressRefused(_)
        ));
    }

    #[test]
    fn endpoint_execute_port_scoped_allow_gates_the_transport() {
        // The full adapter path: a private host allowlisted on :7777 reaches the transport on
        // 7777 but NOT on 9999 (PanicTransport would panic if a refused dial reached it).
        let body = r#"{"head":{"vars":[]},"results":{"bindings":[]}}"#;
        let guard = EgressGuard::deny_private().allow_host("127.0.0.1:7777");
        let ep_ok = Endpoint::with_guard(
            "http://127.0.0.1:7777/sparql",
            Box::new(CannedTransport::new(body)),
            guard.clone(),
        );
        assert_eq!(ep_ok.execute(&SubQuery::new("ASK {}")).unwrap(), body);

        let ep_bad =
            Endpoint::with_guard("http://127.0.0.1:9999/sparql", Box::new(PanicTransport), guard);
        assert!(matches!(
            ep_bad.execute(&SubQuery::new("ASK {}")).unwrap_err(),
            FedError::EgressRefused(_)
        ));
    }

    #[test]
    fn check_addr_is_port_scoped() {
        // The per-address hook: an allowlisted-on-:8053 private IP is dialable on 8053, refused on
        // 8080. A public IP is always dialable regardless of port (the guard only gates private).
        let g = EgressGuard::deny_private().allow_host("127.0.0.1:8053");
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(g.check_addr("127.0.0.1", 8053, lo).is_ok());
        assert!(g.check_addr("127.0.0.1", 8080, lo).is_err());
        let public: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(g.check_addr("8.8.8.8", 12345, public).is_ok());
    }

    // ── capability defaults ───────────────────────────────────────────────────────

    #[test]
    fn capability_defaults_per_interface() {
        let e = Capability::endpoint();
        assert_eq!(e.interface, Interface::Endpoint);
        assert_eq!(e.bind_join, BindJoin::Values);
        assert!(matches!(e.pushable_filters, FilterClass::Full));
        assert!(e.aggregates && e.property_paths && e.order_limit);

        let l = Capability::local();
        assert_eq!(l.interface, Interface::LocalEngine);

        let b = Capability::brtpf(50);
        assert_eq!(b.interface, Interface::BrTpf);
        assert_eq!(b.bind_join, BindJoin::MaxMpR(50));
        assert!(!b.aggregates && !b.property_paths && !b.order_limit);

        let t = Capability::tpf();
        assert_eq!(t.interface, Interface::Tpf);
        assert_eq!(t.bind_join, BindJoin::None);
    }

    // ── Local stub reports Unsupported; fragment sources route execute → solutions ──

    #[test]
    fn fragment_execute_routes_to_solutions_and_local_is_stub() {
        // A fragment source's `execute` (the SRJ entry point) is intentionally Unsupported:
        // it points the caller at the typed `solutions` method (honest, no SRJ overclaim).
        let br = BrTpfSource::new("http://ex/brtpf", 30, Box::new(FixtureFragments::empty()));
        assert!(matches!(
            br.execute(&SubQuery::new("x")).unwrap_err(),
            FedError::Unsupported(_)
        ));
        assert!(matches!(br.source_type(), SourceType::BrTpf(_)));

        let t = TpfSource::new("http://ex/tpf", Box::new(FixtureFragments::empty()));
        assert!(matches!(
            t.execute(&SubQuery::new("x")).unwrap_err(),
            FedError::Unsupported(_)
        ));
        assert!(matches!(t.source_type(), SourceType::Tpf(_)));

        let lo = LocalSource { id: "local".into() };
        assert!(matches!(
            lo.execute(&SubQuery::new("x")).unwrap_err(),
            FedError::Unsupported(_)
        ));
        assert!(matches!(lo.source_type(), SourceType::Local(_)));
        assert_eq!(lo.discover().unwrap().0.interface, Interface::LocalEngine);
    }

    #[test]
    fn endpoint_discover_is_capability_stub_with_no_descriptor() {
        let ep = Endpoint::new(
            "http://8.8.8.8/sparql",
            Box::new(CannedTransport::new("{}")),
        );
        let (cap, desc) = ep.discover().unwrap();
        assert_eq!(cap.interface, Interface::Endpoint);
        assert!(
            desc.is_none(),
            "Phase-2 discover must NOT fabricate a descriptor"
        );
        assert!(matches!(ep.source_type(), SourceType::Endpoint(_)));
        assert_eq!(ep.url(), "http://8.8.8.8/sparql");
    }

    // ── Phase 6: a fixture TPF/brTPF server + the completeness invariant ──────────────

    /// An in-memory fixture fragment server: a fixed set of triples, paginated at
    /// `page_size`, that answers a single triple pattern (plain TPF) AND a brTPF binding
    /// block. It is a REAL server model — it computes the matching triples itself (no canned
    /// rows), follows the same match semantics a conformant TPF/brTPF server would, reports a
    /// truthful `hydra:totalItems`, and records every request it saw so a test can assert the
    /// `maxMpR`-bounded block discipline. [OPUS-4.8] sq-2qze.
    struct FixtureFragments {
        triples: Vec<FragTriple>,
        page_size: usize,
        /// Recorded (binding-block-size, page-token) per request, for block-discipline asserts.
        requests: Mutex<Vec<(usize, Option<String>)>>,
    }

    impl FixtureFragments {
        fn empty() -> Self {
            FixtureFragments {
                triples: Vec::new(),
                page_size: 100,
                requests: Mutex::new(Vec::new()),
            }
        }
        fn new(triples: Vec<FragTriple>, page_size: usize) -> Self {
            FixtureFragments {
                triples,
                page_size: page_size.max(1),
                requests: Mutex::new(Vec::new()),
            }
        }

        /// Whether a stored `triple` matches the requested `pattern` (a bound position must
        /// equal; a variable position is a wildcard, with repeated-variable consistency).
        fn matches_pattern(pattern: &FragPattern, triple: &FragTriple) -> bool {
            bind_triple(pattern, triple).is_some()
        }

        /// Whether `triple` joins at least one mapping in the brTPF `bindings` block: i.e.
        /// substituting a binding into the pattern's variables yields a pattern this triple
        /// satisfies. An empty block means "no binding filter" (plain TPF).
        fn joins_a_binding(
            pattern: &FragPattern,
            triple: &FragTriple,
            bindings: &[FragBinding],
        ) -> bool {
            if bindings.is_empty() {
                return true;
            }
            bindings.iter().any(|b| {
                // The triple already satisfies `pattern`; it joins binding `b` iff the
                // variable values `b` assigns are consistent with the values the triple binds.
                match bind_triple(pattern, triple) {
                    None => false,
                    Some(row) => b.iter().all(|(name, val)| {
                        row.iter()
                            .find(|(n, _)| n == name)
                            .map(|(_, v)| v == val)
                            // a binding var not in this pattern does not constrain the triple
                            .unwrap_or(true)
                    }),
                }
            })
        }
    }

    impl FragmentTransport for FixtureFragments {
        fn fetch_fragment(
            &self,
            _url: &str,
            pattern: &FragPattern,
            bindings: &[FragBinding],
            page: Option<&str>,
        ) -> Result<FragmentPage, String> {
            self.requests
                .lock()
                .unwrap()
                .push((bindings.len(), page.map(str::to_string)));

            // All triples this fragment (pattern ∧ optional binding block) matches.
            let matched: Vec<&FragTriple> = self
                .triples
                .iter()
                .filter(|t| Self::matches_pattern(pattern, t))
                .filter(|t| Self::joins_a_binding(pattern, t, bindings))
                .collect();
            let total_items = matched.len() as u64;

            // Page offset comes from the token ("offset:<n>"); first page = 0.
            let offset = match page {
                None => 0,
                Some(tok) => tok
                    .strip_prefix("offset:")
                    .and_then(|n| n.parse::<usize>().ok())
                    .ok_or_else(|| format!("fixture: bad page token {tok:?}"))?,
            };
            let end = (offset + self.page_size).min(matched.len());
            let triples: Vec<FragTriple> =
                matched[offset..end].iter().map(|t| (*t).clone()).collect();
            let next = if end < matched.len() {
                Some(format!("offset:{end}"))
            } else {
                None
            };
            Ok(FragmentPage {
                triples,
                total_items,
                next,
            })
        }
    }

    /// A small backing graph: people who `knows` each other + a couple of `name`s.
    fn fixture_triples() -> Vec<FragTriple> {
        let knows = || FragTerm::iri("http://xmlns.com/foaf/0.1/knows");
        let name = || FragTerm::iri("http://xmlns.com/foaf/0.1/name");
        let p = |n: &str| FragTerm::iri(format!("http://ex/{n}"));
        let lit = |s: &str| FragTerm::Literal(format!("\"{s}\""));
        vec![
            FragTriple::new(p("alice"), knows(), p("bob")),
            FragTriple::new(p("alice"), knows(), p("carol")),
            FragTriple::new(p("bob"), knows(), p("carol")),
            FragTriple::new(p("carol"), knows(), p("dave")),
            FragTriple::new(p("alice"), name(), lit("Alice")),
            FragTriple::new(p("bob"), name(), lit("Bob")),
        ]
    }

    /// `?s foaf:knows ?o` — predicate bound, subject + object variable.
    fn knows_pattern() -> FragPattern {
        FragPattern::new(
            PatternTerm::Var("s".into()),
            PatternTerm::Bound(FragTerm::iri("http://xmlns.com/foaf/0.1/knows")),
            PatternTerm::Var("o".into()),
        )
    }

    /// Sort bindings into a canonical form for multiset equality (order-independent).
    fn canon(mut rows: Vec<FragBinding>) -> Vec<Vec<(String, FragTerm)>> {
        for r in &mut rows {
            r.sort_by(|a, b| a.0.cmp(&b.0));
        }
        rows.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        rows
    }

    #[test]
    fn plain_tpf_returns_complete_fragment_across_pages() {
        // page_size 2 forces pagination over the 4 knows-triples → exercises hydra:next.
        let server = FixtureFragments::new(fixture_triples(), 2);
        let tpf = TpfSource::new("http://frag/tpf", Box::new(server));
        let pat = knows_pattern();
        let got = tpf.solutions(&pat).unwrap();

        // The complete answer: every knows-triple bound into (?s, ?o), all 4, no duplicates.
        let p = |n: &str| FragTerm::iri(format!("http://ex/{n}"));
        let expect = vec![
            vec![("s".to_string(), p("alice")), ("o".to_string(), p("bob"))],
            vec![("s".to_string(), p("alice")), ("o".to_string(), p("carol"))],
            vec![("s".to_string(), p("bob")), ("o".to_string(), p("carol"))],
            vec![("s".to_string(), p("carol")), ("o".to_string(), p("dave"))],
        ];
        assert_eq!(
            canon(got),
            canon(expect),
            "plain TPF must return COMPLETE fragment"
        );
    }

    #[test]
    fn plain_tpf_cardinality_is_count_metadata() {
        let server = FixtureFragments::new(fixture_triples(), 2);
        let tpf = TpfSource::new("http://frag/tpf", Box::new(server));
        // hydra:totalItems for ?s knows ?o is the 4 matching triples (NOT a page slice).
        assert_eq!(tpf.cardinality(&knows_pattern()).unwrap(), 4);
        // discover() surfaces a one-pattern descriptor seeded with the OPEN-pattern count (6).
        let (cap, desc) = tpf.discover().unwrap();
        assert_eq!(cap.interface, Interface::Tpf);
        let d = desc.expect("count-metadata descriptor");
        assert_eq!(
            d.total_triples, 6,
            "open-pattern hydra:totalItems = all 6 triples"
        );
    }

    #[test]
    fn brtpf_bind_join_is_complete_and_respects_max_mpr() {
        // Upstream bindings for ?s: alice, bob, carol, dave — pushed as a brTPF block.
        let p = |n: &str| FragTerm::iri(format!("http://ex/{n}"));
        let bindings: Vec<FragBinding> = ["alice", "bob", "carol", "dave"]
            .iter()
            .map(|n| vec![("s".to_string(), p(n))])
            .collect();

        // maxMpR = 2 → the 4 bindings MUST be issued as 2 blocks of 2 (not one block of 4).
        let server = FixtureFragments::new(fixture_triples(), 100);
        let brtpf = BrTpfSource::new("http://frag/brtpf", 2, Box::new(server));
        let pat = knows_pattern();
        let got = brtpf.solutions(&pat, &bindings).unwrap();

        // Complete bind-join: every (?s, ?o) where ?s ∈ {alice,bob,carol,dave} AND the triple
        // exists — i.e. ALL 4 knows-triples (dave has no outgoing knows, contributes nothing).
        let expect = vec![
            vec![("s".to_string(), p("alice")), ("o".to_string(), p("bob"))],
            vec![("s".to_string(), p("alice")), ("o".to_string(), p("carol"))],
            vec![("s".to_string(), p("bob")), ("o".to_string(), p("carol"))],
            vec![("s".to_string(), p("carol")), ("o".to_string(), p("dave"))],
        ];
        assert_eq!(
            canon(got),
            canon(expect),
            "brTPF bind-join must be COMPLETE over all maxMpR blocks"
        );
        // Block discipline (the maxMpR bound) is asserted in
        // `brtpf_issues_max_mpr_bounded_blocks` via a recording transport.
    }

    #[test]
    fn brtpf_issues_max_mpr_bounded_blocks() {
        // A recording server lets us assert the EXACT block sizes issued (the maxMpR bound).
        let p = |n: &str| FragTerm::iri(format!("http://ex/{n}"));
        let bindings: Vec<FragBinding> = ["alice", "bob", "carol", "dave", "alice"]
            .iter()
            .map(|n| vec![("s".to_string(), p(n))])
            .collect();
        let server = FixtureFragments::new(fixture_triples(), 100);
        // Keep a handle on the request log by constructing the server, wrapping in the adapter,
        // and reading the Mutex after — so use an Arc-shared recorder.
        use std::sync::Arc;
        struct SharedRecorder {
            inner: FixtureFragments,
        }
        impl FragmentTransport for Arc<SharedRecorder> {
            fn fetch_fragment(
                &self,
                url: &str,
                pattern: &FragPattern,
                bindings: &[FragBinding],
                page: Option<&str>,
            ) -> Result<FragmentPage, String> {
                self.inner.fetch_fragment(url, pattern, bindings, page)
            }
        }
        let rec = Arc::new(SharedRecorder { inner: server });
        let brtpf = BrTpfSource::new("http://frag/brtpf", 2, Box::new(Arc::clone(&rec)));
        let _ = brtpf.solutions(&knows_pattern(), &bindings).unwrap();

        let reqs = rec.inner.requests.lock().unwrap();
        // 5 bindings, maxMpR 2 → blocks of [2, 2, 1]; one request each (page_size 100, single
        // page per block). Every block size MUST be ≤ 2 (the maxMpR bound, never exceeded).
        let block_sizes: Vec<usize> = reqs.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            block_sizes,
            vec![2, 2, 1],
            "maxMpR=2 → 3 blocks sized [2,2,1]"
        );
        assert!(
            block_sizes.iter().all(|n| *n <= 2),
            "no request may exceed maxMpR=2"
        );
    }

    #[test]
    fn brtpf_empty_bindings_degrades_to_unbound_scan() {
        // No upstream bindings → one unbound fragment scan, still complete (= plain TPF).
        let server = FixtureFragments::new(fixture_triples(), 100);
        let brtpf = BrTpfSource::new("http://frag/brtpf", 50, Box::new(server));
        let got = brtpf.solutions(&knows_pattern(), &[]).unwrap();
        assert_eq!(
            got.len(),
            4,
            "unbound brTPF scan returns the whole fragment"
        );
    }

    #[test]
    fn brtpf_discover_reports_max_mpr_and_open_count() {
        let server = FixtureFragments::new(fixture_triples(), 100);
        let brtpf = BrTpfSource::new("http://frag/brtpf", 50, Box::new(server));
        let (cap, desc) = brtpf.discover().unwrap();
        assert_eq!(cap.bind_join, BindJoin::MaxMpR(50));
        assert_eq!(cap.interface, Interface::BrTpf);
        // The count-metadata descriptor is keyed to the OPEN pattern (all 6 triples).
        assert_eq!(desc.unwrap().total_triples, 6);
    }

    #[test]
    fn bind_triple_rejects_nonmatching_and_handles_self_join() {
        // A misbehaving server returning a triple that violates a bound slot yields no row
        // (the load-bearing answer-safety check — a wrong triple can't smuggle a binding).
        let pat = knows_pattern(); // predicate bound to foaf:knows
        let wrong = FragTriple::new(
            FragTerm::iri("http://ex/alice"),
            FragTerm::iri("http://ex/NOT-knows"),
            FragTerm::iri("http://ex/bob"),
        );
        assert!(
            bind_triple(&pat, &wrong).is_none(),
            "bound predicate mismatch ⇒ no row"
        );

        // Self-join pattern ?x knows ?x only matches a self-loop triple.
        let selfp = FragPattern::new(
            PatternTerm::Var("x".into()),
            PatternTerm::Bound(FragTerm::iri("http://xmlns.com/foaf/0.1/knows")),
            PatternTerm::Var("x".into()),
        );
        let not_loop = FragTriple::new(
            FragTerm::iri("http://ex/alice"),
            FragTerm::iri("http://xmlns.com/foaf/0.1/knows"),
            FragTerm::iri("http://ex/bob"),
        );
        assert!(
            bind_triple(&selfp, &not_loop).is_none(),
            "?x..?x rejects a non-loop"
        );
        let loop_t = FragTriple::new(
            FragTerm::iri("http://ex/alice"),
            FragTerm::iri("http://xmlns.com/foaf/0.1/knows"),
            FragTerm::iri("http://ex/alice"),
        );
        let row = bind_triple(&selfp, &loop_t).expect("self-loop binds ?x once");
        assert_eq!(
            row,
            vec![("x".to_string(), FragTerm::iri("http://ex/alice"))]
        );
    }

    #[test]
    fn chunk_bindings_caps_block_size_and_handles_degenerate_cap() {
        let mk = |n: usize| -> Vec<FragBinding> {
            (0..n)
                .map(|i| vec![("s".to_string(), FragTerm::iri(format!("http://ex/{i}")))])
                .collect()
        };
        let b = mk(5);
        let blocks = chunk_bindings(&b, 2);
        assert_eq!(
            blocks.iter().map(|x| x.len()).collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
        // maxMpR 0 is treated as 1 (never a zero-sized block ⇒ no infinite loop).
        let blocks0 = chunk_bindings(&b, 0);
        assert!(blocks0.iter().all(|x| x.len() == 1));
        assert_eq!(blocks0.len(), 5);
        // Empty input ⇒ no blocks.
        assert!(chunk_bindings(&[], 4).is_empty());
    }

    // ── Native HttpTransport: the SSRF resolver PINS the vetted IP (sq-25xk) ──────────
    //
    // These prove the load-bearing TOCTOU fix: the egress policy lives INSIDE ureq's own
    // resolver, so ureq dials only the addresses the resolver returns. The classifier tests
    // above prove WHICH IPs are forbidden; these prove the *resolver ureq actually calls*
    // applies that classification — including the host-authority parse, the IPv6 bracket
    // strip, and the allowlist bypass. Every case uses an IP-LITERAL netloc so `to_socket_addrs`
    // is hermetic (no DNS lookup, no network), so the tests are deterministic. [OPUS-4.8] sq-25xk.

    #[cfg(not(target_arch = "wasm32"))]
    fn resolver_with(allow: &[&str]) -> EgressFilterResolver {
        let set: HashSet<String> = allow.iter().map(|h| h.to_ascii_lowercase()).collect();
        EgressFilterResolver {
            allow_private: std::sync::Arc::new(set),
        }
    }

    /// [OPUS-4.8] sq-g2xs: invoke the ureq-3 `Resolver` for a `host:port` netloc by building the
    /// `http://<netloc>/` URI the resolver parses, with a default `Config` + no-deadline timeout.
    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_netloc(
        r: &EgressFilterResolver,
        netloc: &str,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        use ureq::unversioned::resolver::Resolver;
        let uri: ureq::http::Uri = format!("http://{netloc}/").parse().unwrap();
        let config = ureq::Agent::config_builder().build();
        // The resolver ignores the timeout; build a no-deadline one.
        let timeout = ureq::unversioned::transport::NextTimeout {
            after: ureq::unversioned::transport::time::Duration::NotHappening,
            reason: ureq::Timeout::Global,
        };
        r.resolve(&uri, &config, timeout)
    }

    /// `true` iff `e` is the egress-refusal `PermissionDenied` io error.
    #[cfg(not(target_arch = "wasm32"))]
    fn is_permission_denied(e: &ureq::Error) -> bool {
        matches!(e, ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied)
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolver_refuses_private_metadata_and_loopback_literals() {
        // The default-deny resolver must refuse every private/internal IP literal — ureq is
        // never handed a dialable address (so a re-bound DNS answer cannot connect to it).
        for netloc in [
            "10.0.0.5:443",          // RFC1918
            "127.0.0.1:8080",        // loopback
            "169.254.169.254:80",    // cloud metadata (link-local)
            "192.168.1.1:80",        // RFC1918
            "100.64.0.1:80",         // CGNAT
            "[::1]:80",              // IPv6 loopback (bracketed)
            "[fc00::1]:80",          // IPv6 unique-local (bracketed)
            "[::ffff:127.0.0.1]:80", // v4-mapped loopback can't smuggle through a v6 literal
        ] {
            match resolve_netloc(&resolver_with(&[]), netloc) {
                Ok(addrs) => panic!("{netloc} should have been refused, got {addrs:?}"),
                Err(e) => assert!(
                    is_permission_denied(&e),
                    "{netloc} must be refused with PermissionDenied, got {e:?}"
                ),
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolver_permits_public_literal() {
        // A public address survives the filter and is handed back to ureq to dial.
        let addrs = resolve_netloc(&resolver_with(&[]), "8.8.8.8:80")
            .expect("a public IP must be permitted by the egress resolver");
        assert!(
            addrs.iter().any(|sa| sa.ip().to_string() == "8.8.8.8"),
            "the public address must be returned for ureq to dial"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolver_allowlist_reopens_private_literal() {
        // An allowlisted private host is permitted even though it resolves privately — the
        // allowlist bridged from the EgressGuard re-opens it at the socket-resolution layer.
        let addrs = resolve_netloc(&resolver_with(&["10.0.0.5"]), "10.0.0.5:443")
            .expect("an allowlisted private host must be permitted");
        assert!(addrs.iter().any(|sa| sa.ip().to_string() == "10.0.0.5"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn from_guard_bridges_allowlist_into_resolver() {
        // The transport built from a guard shares the guard's allowlist, so an allowlisted
        // private host the guard would permit is ALSO permitted by the transport's resolver
        // (the two resolutions agree — no second, unguarded re-resolve).
        let guard = EgressGuard::deny_private().allow_host("sparql.internal");
        let t = HttpTransport::from_guard(&guard);
        assert!(t.allow_private.contains("sparql.internal"));
        // A default-deny guard yields an empty transport allowlist.
        let empty = HttpTransport::from_guard(&EgressGuard::deny_private());
        assert!(empty.allow_private.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn endpoint_native_denies_loopback_at_preflight() {
        // The native constructor still runs the pre-flight check_endpoint FIRST: a loopback
        // endpoint is refused before the request, with no network round-trip. (The transport's
        // resolver is the SECOND line of defence against a re-bind on a public-looking host.)
        let ep = Endpoint::native("http://127.0.0.1:9999/sparql");
        let err = ep.execute(&SubQuery::new("ASK {}")).unwrap_err();
        assert!(matches!(err, FedError::EgressRefused(_)), "got {err:?}");
    }

    // ── Native HTTP FragmentTransport: Hydra URI-template + Turtle/TriG parse (sq-yzca) ──
    //
    // These prove the four pieces the bead names WITHOUT a network: the Hydra query-string
    // serialisation, the brTPF `values` text-wire attachment, the Turtle/TriG fragment-body
    // parse (control/data split + hydra:totalItems + hydra:next extraction), and that a control
    // triple is never mistaken for a data match. The SSRF resolver itself is the SAME
    // `EgressFilterResolver` the SRJ transport tests above exercise. [OPUS-4.8] sq-yzca.

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fragment_first_page_url_serialises_bound_positions() {
        // ?s foaf:knows ?o — predicate bound, subject + object variable. Only the bound
        // predicate appears in the query string, percent-encoded as an N-Triples IRI.
        let url = HttpFragmentTransport::first_page_url("http://frag/tpf", &knows_pattern(), &[]);
        assert_eq!(
            url,
            "http://frag/tpf?predicate=%3Chttp%3A%2F%2Fxmlns.com%2Ffoaf%2F0.1%2Fknows%3E"
        );
        // A fully-bound pattern serialises all three positions; a base URL that already has a
        // query string gets `&`-joined.
        let bound = FragPattern::new(
            PatternTerm::Bound(FragTerm::iri("http://ex/alice")),
            PatternTerm::Bound(FragTerm::iri("http://xmlns.com/foaf/0.1/knows")),
            PatternTerm::Bound(FragTerm::iri("http://ex/bob")),
        );
        let url2 = HttpFragmentTransport::first_page_url("http://frag/tpf?dataset=x", &bound, &[]);
        assert!(url2.starts_with("http://frag/tpf?dataset=x&subject="));
        assert!(url2.contains("&predicate="));
        assert!(url2.contains("&object="));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fragment_first_page_url_attaches_brtpf_values_block() {
        // A non-empty binding block rides the `values` parameter using the server's text wire.
        let p = |n: &str| FragTerm::iri(format!("http://ex/{n}"));
        let bindings: Vec<FragBinding> = vec![
            vec![("s".to_string(), p("alice"))],
            vec![("s".to_string(), p("bob"))],
        ];
        let url =
            HttpFragmentTransport::first_page_url("http://frag/brtpf", &knows_pattern(), &bindings);
        assert!(url.contains("predicate="), "the bound predicate is present");
        assert!(url.contains("&values="), "the brTPF block rides `values`");
        // The encoded `values` payload decodes back to the server's text wire (s=<…>\ns=<…>).
        let encoded = url.split("values=").nth(1).unwrap();
        let decoded = pct_decode_for_test(encoded);
        assert_eq!(decoded, "s=<http://ex/alice>\ns=<http://ex/bob>");
    }

    /// Decode a percent-encoded string (test helper — the inverse of `pct_encode`). [OPUS-4.8].
    #[cfg(not(target_arch = "wasm32"))]
    fn pct_decode_for_test(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hi = (bytes[i + 1] as char).to_digit(16).unwrap();
                let lo = (bytes[i + 2] as char).to_digit(16).unwrap();
                out.push((hi * 16 + lo) as u8);
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parse_turtle_fragment_splits_control_from_data() {
        // A realistic TPF fragment Turtle body: two data triples + the Hydra/VoID control node
        // (totalItems + next). The parse must return ONLY the data triples that match the
        // pattern, the count, and the next-page link.
        let body = r#"
@prefix hydra: <http://www.w3.org/ns/hydra/core#> .
@prefix void: <http://rdfs.org/ns/void#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
<http://ex/alice> foaf:knows <http://ex/bob> .
<http://ex/alice> foaf:knows <http://ex/carol> .
<http://frag/tpf?predicate=knows&page=0> void:triples 4 ;
    hydra:totalItems 4 ;
    hydra:itemsPerPage 2 ;
    hydra:next <http://frag/tpf?predicate=knows&page=1> .
"#;
        let page = parse_fragment_body(body, &knows_pattern()).unwrap();
        assert_eq!(page.total_items, 4, "hydra:totalItems / void:triples");
        assert_eq!(
            page.next.as_deref(),
            Some("http://frag/tpf?predicate=knows&page=1"),
            "hydra:next page link"
        );
        // Exactly the two matching knows-triples — the control triples are excluded.
        assert_eq!(page.triples.len(), 2, "only the data triples survive");
        let p = |n: &str| FragTerm::iri(format!("http://ex/{n}"));
        let knows = FragTerm::iri("http://xmlns.com/foaf/0.1/knows");
        assert!(page
            .triples
            .contains(&FragTriple::new(p("alice"), knows.clone(), p("bob"))));
        assert!(page
            .triples
            .contains(&FragTriple::new(p("alice"), knows, p("carol"))));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parse_trig_fragment_named_graph_body() {
        // A TriG body wrapping the data + controls in a named graph still parses (TriG superset
        // of Turtle): the count, next link, and data triples come through identically.
        let body = r#"
@prefix hydra: <http://www.w3.org/ns/hydra/core#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
<http://frag/g> {
  <http://ex/bob> foaf:knows <http://ex/carol> .
  <http://frag/tpf?page=0> hydra:totalItems 1 .
}
"#;
        let page = parse_fragment_body(body, &knows_pattern()).unwrap();
        assert_eq!(page.total_items, 1);
        assert!(page.next.is_none(), "no hydra:next ⇒ last page");
        assert_eq!(page.triples.len(), 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parse_fragment_filters_nonmatching_data_triple() {
        // A misbehaving server returns a triple that does NOT match the requested pattern (a
        // different predicate). The match filter drops it — it can never become a binding.
        let body = r#"
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
<http://ex/alice> foaf:knows <http://ex/bob> .
<http://ex/alice> foaf:name "Alice" .
"#;
        // Pattern is ?s foaf:knows ?o — the foaf:name triple must be filtered out.
        let page = parse_fragment_body(body, &knows_pattern()).unwrap();
        assert_eq!(page.triples.len(), 1, "non-matching predicate dropped");
        assert_eq!(
            page.triples[0].predicate,
            FragTerm::iri("http://xmlns.com/foaf/0.1/knows")
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parse_malformed_fragment_is_clean_error() {
        // A TriG syntax error is a clean transport-error string (never a panic — forbid unsafe).
        let err = parse_fragment_body("@prefix x: <broken .", &knows_pattern()).unwrap_err();
        assert!(err.contains("malformed"), "got {err:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn http_fragment_transport_from_guard_bridges_allowlist() {
        // The transport built from a guard shares the guard's allowlist (so the SSRF resolver and
        // the pre-flight egress check agree — one source of truth, no second unguarded resolve).
        let guard = EgressGuard::deny_private().allow_host("frag.internal");
        let t = HttpFragmentTransport::from_guard(&guard);
        assert!(t.allow_private.contains("frag.internal"));
        let empty = HttpFragmentTransport::from_guard(&EgressGuard::deny_private());
        assert!(empty.allow_private.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn endpoint_native_preserves_guard_and_url() {
        // with_guard_native keeps the supplied guard (so the pre-flight allowlist is intact)
        // AND wires a transport whose resolver shares that same allowlist.
        let guard = EgressGuard::deny_private().allow_host("sparql.internal");
        let ep = Endpoint::with_guard_native("http://sparql.internal/sparql", guard);
        assert_eq!(ep.url(), "http://sparql.internal/sparql");
        assert!(ep.guard().is_allowed("sparql.internal"));
        // An allowlisted host short-circuits the pre-flight resolution and returns Ok(host).
        assert_eq!(
            ep.guard()
                .check_endpoint("http://sparql.internal/sparql")
                .unwrap(),
            "sparql.internal"
        );
    }

    // [FABLE-5] sq-3dyje.6 (mutation-kill): the fragment-body control/data split must set
    // `next` from EXACTLY the hydra:next / legacy hydra:nextPage predicates — cargo-mutants
    // showed `pred == …nextPage` mutated to `!=` survived, i.e. no test pinned that another
    // control link (hydra:first / hydra:last also pass is_control_predicate and carry
    // NamedNode objects) must NOT become the pagination cursor. Walking a wrong "next" link
    // would re-fetch page 1 forever or truncate the fragment — a real answer-safety bug class.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fragment_body_control_links_other_than_next_do_not_paginate() {
        let pattern = FragPattern::new(
            PatternTerm::Var("s".to_string()),
            PatternTerm::Bound(FragTerm::Iri("http://ex/p".to_string())),
            PatternTerm::Var("o".to_string()),
        );
        // hydra:first + hydra:last are control links with IRI objects, but NOT next-page
        // cursors; hydra:totalItems carries the count; one matching data triple.
        let body = concat!(
            "<http://frag/page1> <http://www.w3.org/ns/hydra/core#first> <http://frag/page1> .\n",
            "<http://frag/page1> <http://www.w3.org/ns/hydra/core#last> <http://frag/page9> .\n",
            "<http://frag/page1> <http://www.w3.org/ns/hydra/core#totalItems> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/a> <http://ex/p> <http://ex/b> .\n",
        );
        let page = parse_fragment_body(body, &pattern).expect("well-formed fragment parses");
        assert_eq!(
            page.next, None,
            "hydra:first/last are NOT pagination cursors — only hydra:next/nextPage are"
        );
        assert_eq!(page.total_items, 42, "hydra:totalItems drives the count");
        assert_eq!(page.triples.len(), 1, "exactly the one matching data triple");
        assert_eq!(
            page.triples[0],
            FragTriple::new(
                FragTerm::Iri("http://ex/a".to_string()),
                FragTerm::Iri("http://ex/p".to_string()),
                FragTerm::Iri("http://ex/b".to_string()),
            )
        );
    }

    // [FABLE-5] sq-3dyje.6 (mutation-kill): each arm of is_control_predicate's `||` chain is
    // INDEPENDENTLY load-bearing. cargo-mutants showed `||`→`&&` survive because no test
    // isolates an arm: a document whose only non-data triple is classified by JUST ONE arm.
    // Mutating any single arm to `&&` makes that predicate fail the whole (now-conjunctive)
    // test, so the triple leaks into the data set and inflates the pattern-matched count.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn each_control_predicate_arm_is_independently_load_bearing() {
        // A permissive pattern (all vars) so ANY triple that reaches the data path matches and
        // is counted — making a mis-classified control triple observable as an extra data row.
        let pattern = FragPattern::new(
            PatternTerm::Var("s".to_string()),
            PatternTerm::Var("p".to_string()),
            PatternTerm::Var("o".to_string()),
        );
        // Three fragments, each carrying EXACTLY one control triple classified by ONE arm, plus
        // one genuine data triple. Only the data triple may survive the control/data split.
        let cases = [
            // hydra: arm (starts_with HYDRA_NS)
            (
                "<http://frag> <http://www.w3.org/ns/hydra/core#itemsPerPage> \"10\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                "hydra:",
            ),
            // void: arm (starts_with VOID_NS)
            (
                "<http://frag> <http://rdfs.org/ns/void#triples> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
                "void:",
            ),
            // rdf:type arm (exact equality)
            (
                "<http://frag> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/hydra/core#Collection> .\n",
                "rdf:type",
            ),
        ];
        for (control_triple, arm) in cases {
            let body = format!(
                "{control_triple}<http://ex/a> <http://ex/p> <http://ex/b> .\n"
            );
            let page = parse_fragment_body(&body, &pattern).expect("well-formed fragment parses");
            assert_eq!(
                page.triples.len(),
                1,
                "the {} control triple must NOT be counted as data — only the one data triple",
                arm
            );
            assert_eq!(
                page.triples[0],
                FragTriple::new(
                    FragTerm::Iri("http://ex/a".to_string()),
                    FragTerm::Iri("http://ex/p".to_string()),
                    FragTerm::Iri("http://ex/b".to_string()),
                ),
                "the surviving triple is the genuine data triple, not the {} control node",
                arm
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fragment_body_legacy_next_page_predicate_paginates() {
        // The legacy hydra:nextPage spelling must set the cursor exactly like hydra:next.
        let pattern = FragPattern::new(
            PatternTerm::Var("s".to_string()),
            PatternTerm::Var("p".to_string()),
            PatternTerm::Var("o".to_string()),
        );
        let body =
            "<http://frag/page1> <http://www.w3.org/ns/hydra/core#nextPage> <http://frag/page2> .\n";
        let page = parse_fragment_body(body, &pattern).expect("well-formed fragment parses");
        assert_eq!(page.next.as_deref(), Some("http://frag/page2"));
        assert_eq!(page.total_items, 0, "no count metadata in this fragment");
        assert!(page.triples.is_empty(), "control-only fragment has no data");
    }

    // [FABLE-5] sq-3dyje.6 (mutation-kill): the count comes from EITHER hydra:totalItems OR
    // void:triples. The earlier fragment tests read the count from hydra:totalItems only, so
    // the `pred == void:triples` half of the count predicate could be mutated to `!=` unnoticed
    // (a void:triples-only fragment would then report count 0). Pin BOTH count predicates, and
    // the recall-safe MAX when a fragment carries both.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fragment_body_count_reads_void_triples_and_takes_max() {
        let pattern = FragPattern::new(
            PatternTerm::Var("s".to_string()),
            PatternTerm::Var("p".to_string()),
            PatternTerm::Var("o".to_string()),
        );
        // void:triples alone drives the count.
        let void_only =
            "<http://frag> <http://rdfs.org/ns/void#triples> \"77\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n";
        assert_eq!(
            parse_fragment_body(void_only, &pattern).unwrap().total_items,
            77,
            "void:triples must drive the count (kills the void== →!= survivor)"
        );
        // Both present: the recall-safe MAX wins regardless of order.
        let both = concat!(
            "<http://frag> <http://rdfs.org/ns/void#triples> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://frag> <http://www.w3.org/ns/hydra/core#totalItems> \"90\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        );
        assert_eq!(
            parse_fragment_body(both, &pattern).unwrap().total_items,
            90,
            "the max of the two counts is taken"
        );
    }
}
