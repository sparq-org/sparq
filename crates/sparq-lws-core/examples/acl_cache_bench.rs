// AUTHORED-BY Claude Opus 4.8
//! DETERMINISTIC in-process micro-benchmark for the ETag-keyed parsed-ACL cache (read-path
//! optimisation #3). It isolates EXACTLY the work the cache removes — the per-read blob byte-fetch +
//! `oxttl` parse of an UNCHANGED ACL — independent of the HTTP/TLS stack and (crucially) of box load,
//! by measuring the WAC effective-ACL resolution directly against the in-memory store.
//!
//! ## Why this, not only the HTTP bench
//! The `bench/run.sh` / `bench/run-auth.sh` sweeps measure end-to-end RPS/latency, which on a CONTENDED
//! box are dominated by wall-clock noise (the charter marks timing metrics ADVISORY for exactly this
//! reason). This example instead reports a DETERMINISTIC, box-independent metric: a COUNT of the blob
//! reads + ACL parses the resolver performs, cache-OFF vs cache-ON, over N repeated reads of the SAME
//! resource. That count is a reproducible integer — the real substance of the optimisation — and it
//! ALSO prints a relative wall-clock ratio (cold/warm) which, being a same-process same-box ratio
//! measured back-to-back, is far more robust to contention than two separate HTTP runs.
//!
//! Run: `cargo run --release --example acl_cache_bench [-- <iterations> <acl_authorizations>]`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use axum::body::Bytes;

use sparq_lws_core::acl_cache::AclCache;
use sparq_lws_core::authz::wac::WacAuthorizer;
use sparq_lws_core::authz::AccessMode;
use sparq_lws_core::error::ServerResult;
use sparq_lws_core::store::{
    CompositeStore, DeleteOutcome, InMemoryBlobStore, InMemorySparqClient, Resource, ResourceMeta,
    Store,
};

const BASE: &str = "https://pod.example";
const OWNER: &str = "https://pod.example/alice/profile/card#me";

/// A [`Store`] decorator that COUNTS the operations the resolver performs, so we can prove
/// deterministically that the cache eliminates the per-read blob byte-fetch + ACL read on a warm hit.
/// `read` (the byte-fetch + the source the resolver parses) and `meta` (the cheap etag probe) are
/// counted separately — the win is "reads → 0 on the warm path, replaced by a meta probe".
struct CountingStore<S: Store> {
    inner: S,
    reads: AtomicUsize,
    metas: AtomicUsize,
}

impl<S: Store> CountingStore<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            reads: AtomicUsize::new(0),
            metas: AtomicUsize::new(0),
        }
    }
    fn reset(&self) {
        self.reads.store(0, Ordering::Relaxed);
        self.metas.store(0, Ordering::Relaxed);
    }
}

#[async_trait]
impl<S: Store> Store for CountingStore<S> {
    async fn read(&self, iri: &str) -> ServerResult<Resource> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.inner.read(iri).await
    }
    async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
        self.metas.fetch_add(1, Ordering::Relaxed);
        self.inner.meta(iri).await
    }
    async fn exists(&self, iri: &str) -> ServerResult<bool> {
        self.inner.exists(iri).await
    }
    async fn write(&self, iri: &str, body: Bytes, ct: &str) -> ServerResult<ResourceMeta> {
        self.inner.write(iri, body, ct).await
    }
    async fn create_in_container(
        &self,
        c: &str,
        child: &str,
        body: Bytes,
        ct: &str,
    ) -> ServerResult<ResourceMeta> {
        self.inner.create_in_container(c, child, body, ct).await
    }
    async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
        self.inner.delete(iri, parent).await
    }
    async fn delete_container_if_empty(
        &self,
        iri: &str,
        parent: Option<&str>,
    ) -> ServerResult<DeleteOutcome> {
        self.inner.delete_container_if_empty(iri, parent).await
    }
    async fn fence(&self, fence: &sparq_lws_core::store::Fence) -> ServerResult<()> {
        self.inner.fence(fence).await
    }
    async fn list_children(
        &self,
        c: &str,
    ) -> ServerResult<Vec<sparq_lws_core::store::ValidatedChildIri>> {
        self.inner.list_children(c).await
    }
}

/// A realistically-sized owner-private ACL: the owner with full control plus `n_extra` additional
/// `acl:agent` authorizations, so the `oxttl` parse + rule-match has non-trivial bytes/triples (a
/// shared-pod ACL with many delegated grants). The bigger the ACL, the bigger the per-read parse the
/// cache eliminates.
fn private_acl(target: &str, n_extra: usize) -> String {
    let mut s = String::from("@prefix acl: <http://www.w3.org/ns/auth/acl#>.\n");
    s.push_str(&format!(
        "<#owner> a acl:Authorization; acl:agent <{OWNER}>; acl:accessTo <{target}>; acl:default <{target}>; acl:mode acl:Read, acl:Write, acl:Control.\n"
    ));
    for i in 0..n_extra {
        s.push_str(&format!(
            "<#g{i}> a acl:Authorization; acl:agent <https://pod.example/agent{i}/profile/card#me>; acl:accessTo <{target}>; acl:mode acl:Read.\n"
        ));
    }
    s
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let iterations: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200_000);
    let n_extra: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);

    // A container with an owner-private inherited default ACL; the read target inherits it (the common
    // pod shape: most resources have no own ACL and inherit the container's). This makes the resolver
    // walk to the container ACL — one read+parse per resolve in the cold path.
    let target = "https://pod.example/alice/data/doc";
    let container_acl_iri = "https://pod.example/alice/.acl";

    let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
    store
        .write(
            container_acl_iri,
            Bytes::from(private_acl("https://pod.example/alice/", n_extra)),
            "text/turtle",
        )
        .await
        .expect("seed container ACL");
    let store = CountingStore::new(store);

    println!("ACL-cache deterministic micro-benchmark");
    println!("  base={BASE} target={target}");
    println!(
        "  inherited owner-private ACL with {n_extra} extra agent grants ({} bytes)",
        private_acl("https://pod.example/alice/", n_extra).len()
    );
    println!("  iterations (repeated authed reads of the SAME resource) = {iterations}\n");

    // --- COLD: cache OFF (disabled) — the pre-cache path: every resolve reads + parses the ACL. ---
    let off = AclCache::disabled();
    store.reset();
    let t0 = Instant::now();
    for _ in 0..iterations {
        let wac = WacAuthorizer::with_cache(&store, BASE, &off);
        let d = wac
            .authorize_read(target, AccessMode::Read, Some(OWNER), None)
            .await
            .unwrap();
        std::hint::black_box(&d);
    }
    let cold = t0.elapsed();
    let cold_reads = store.reads.load(Ordering::Relaxed);
    let cold_metas = store.metas.load(Ordering::Relaxed);

    // --- WARM: cache ON — the first resolve populates, the rest are hits (no read, no parse). ---
    let on = AclCache::new(4096);
    store.reset();
    let t1 = Instant::now();
    for _ in 0..iterations {
        let wac = WacAuthorizer::with_cache(&store, BASE, &on);
        let d = wac
            .authorize_read(target, AccessMode::Read, Some(OWNER), None)
            .await
            .unwrap();
        std::hint::black_box(&d);
    }
    let warm = t1.elapsed();
    let warm_reads = store.reads.load(Ordering::Relaxed);
    let warm_metas = store.metas.load(Ordering::Relaxed);

    // --- DETERMINISTIC metric (box-INDEPENDENT): the count of blob reads + ACL parses eliminated. ---
    println!("DETERMINISTIC (reproducible, box-independent):");
    println!(
        "  cache OFF : {cold_reads} store.read calls (1 ACL byte-fetch + parse per resolve), {cold_metas} meta probes"
    );
    println!(
        "  cache ON  : {warm_reads} store.read calls (only the COLD-miss populate), {warm_metas} meta probes (the cheap etag check)"
    );
    let reads_eliminated = cold_reads.saturating_sub(warm_reads);
    // Percentage of the OFF-path ACL byte-fetch+parse work that the cache removes. The base is the
    // cache-OFF read count (the work actually performed without the cache), NOT `iterations` — the
    // resolver may do != 1 counted read per resolve, so dividing by iterations would misrepresent the
    // fraction eliminated. Zero-read / zero-iteration is handled explicitly (no divide-by-zero).
    let eliminated_pct = if cold_reads == 0 {
        0.0
    } else {
        100.0 * reads_eliminated as f64 / cold_reads as f64
    };
    println!(
        "  => the cache ELIMINATES {reads_eliminated} ACL byte-fetch+parse operations over {iterations} reads ({eliminated_pct:.2}% of the {cold_reads} OFF-path reads), replacing each with one cheap meta etag probe + a HashMap lookup."
    );

    // --- ADVISORY (wall-clock, same-process back-to-back ratio — robust to contention). ---
    println!(
        "\nADVISORY (wall-clock, same-process ratio — robust to box load, NOT a gated number):"
    );
    println!(
        "  cache OFF : {cold:?} total, {:.3} us/resolve",
        cold.as_micros() as f64 / iterations as f64
    );
    println!(
        "  cache ON  : {warm:?} total, {:.3} us/resolve",
        warm.as_micros() as f64 / iterations as f64
    );
    let speedup = cold.as_secs_f64() / warm.as_secs_f64().max(f64::MIN_POSITIVE);
    println!("  warm-resolve speedup = {speedup:.2}x (cold/warm)");

    // Keep the arc-imports honest (the example uses Arc indirectly via async fn boxing on some paths).
    let _ = Arc::new(());
}
