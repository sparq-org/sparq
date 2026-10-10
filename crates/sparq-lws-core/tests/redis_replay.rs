// AUTHORED-BY Claude Opus 4.8
//! Distributed Redis DPoP-`jti` replay store — integration tests proving the DISTRIBUTED correctness
//! the per-instance in-memory store CANNOT provide.
//!
//! Compiled ONLY under the opt-in `redis-replay` feature (so the default `cargo test` gate is
//! unchanged and carries no Redis dependency). The live tests are additionally `#[ignore]` + env-gated
//! on `PSS_IT_REDIS_URL` (e.g. `redis://127.0.0.1:6379`), mirroring the live-SPARQ IT pattern: they run
//! ONLY when a real Redis is up. Bring it up with the repo's `docker compose up -d redis` and run:
//!
//! ```sh
//! PSS_IT_REDIS_URL=redis://127.0.0.1:6379 \
//!   cargo test --features redis-replay --test redis_replay -- --ignored
//! ```
//!
//! Coverage (the whole point — the horizontal-scaling guarantees):
//! - (a) CROSS-INSTANCE replay rejection: a `jti` marked via store A is rejected (`Replay`) via a
//!   SEPARATE store B pointed at the SAME Redis — the property the in-memory store lacks.
//! - (b) FAIL-CLOSED: with Redis unreachable, `mark` returns a `ReplayBackendError` (never `New` —
//!   the auth path then 503s, never silently accepts).
//! - (c) TTL EXPIRY: a `jti` becomes re-markable (`New` again) after its `PX` ttl elapses.
//!
//! The cross-instance + ttl tests use UNIQUE per-run `jti`s so repeated runs against a persistent Redis
//! never collide (and they do not depend on flushing the DB).

#![cfg(feature = "redis-replay")]

use std::time::Duration;

use solid_oidc_verifier::replay::{MarkResult, ReplayStore};
use sparq_lws_core::redis_replay::RedisReplayStore;

const ONE_MINUTE: Duration = Duration::from_secs(60);

/// The live-Redis URL, or `None` to skip (the IT is `#[ignore]`d, so it only runs with `--ignored`).
fn redis_url() -> Option<String> {
    match std::env::var("PSS_IT_REDIS_URL") {
        Ok(u) if !u.trim().is_empty() => Some(u),
        _ => {
            eprintln!("PSS_IT_REDIS_URL not set; skipping live Redis replay test");
            None
        }
    }
}

/// A unique, high-entropy `jti` so a persistent Redis across runs never produces a false replay. Uses
/// the nanosecond clock + a thread-id-ish salt; the keyspace is large enough for a test.
fn unique_jti(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("it-{tag}-{nanos}-{:?}", std::thread::current().id())
}

/// (a) CROSS-INSTANCE replay rejection — the horizontal-scaling correctness the in-memory store LACKS.
/// Two SEPARATE `RedisReplayStore`s (simulating two server instances) share ONE Redis: a `jti` marked
/// New on store A must be reported as a Replay on store B.
#[test]
#[ignore = "needs a live Redis (set PSS_IT_REDIS_URL); run with --ignored"]
fn cross_instance_replay_is_rejected() {
    let Some(url) = redis_url() else { return };

    // Two independent stores = two independent "instances", each with its own worker thread + pool,
    // both pointed at the same Redis.
    let store_a = RedisReplayStore::connect(&url).expect("instance A connects to Redis");
    let store_b = RedisReplayStore::connect(&url).expect("instance B connects to Redis");

    let jti = unique_jti("cross");
    let ttl = Duration::from_secs(60);

    // Instance A sees a FRESH proof.
    assert_eq!(
        store_a.mark(&jti, ttl).expect("A.mark ok"),
        MarkResult::New,
        "first mark of a jti on instance A must be New"
    );

    // Instance B — a DIFFERENT process/store — must catch the replay because the set is SHARED. This is
    // exactly what the per-instance in-memory store cannot do.
    assert_eq!(
        store_b.mark(&jti, ttl).expect("B.mark ok"),
        MarkResult::Replay,
        "a jti marked on instance A MUST be a Replay on instance B (shared Redis set)"
    );

    // And re-marking on A is still a replay (no double-accept anywhere).
    assert_eq!(
        store_a.mark(&jti, ttl).expect("A.re-mark ok"),
        MarkResult::Replay,
        "re-marking the same jti on instance A is still a Replay"
    );

    // A DISTINCT jti is independently fresh on either instance.
    let jti2 = unique_jti("cross2");
    assert_eq!(
        store_b.mark(&jti2, ttl).expect("B.mark distinct ok"),
        MarkResult::New,
        "a distinct jti is fresh"
    );
}

/// (b.1) FAIL-CLOSED at BOOT — with Redis UNREACHABLE, `connect` returns an error so the server refuses
/// to start rather than run with a silently-broken (effectively disabled) shared replay set. No live
/// Redis needed (points at a closed port).
#[test]
fn unreachable_redis_fails_closed_at_connect() {
    // Port 1 is reserved/unused. A tight timeout keeps this fast. The connect MUST error (fail-closed).
    let res =
        RedisReplayStore::connect_with_timeout("redis://127.0.0.1:1", Duration::from_millis(75));
    assert!(
        res.is_err(),
        "connecting to an unreachable Redis MUST fail closed (Err), never Ok — a dead shared store \
         must not let the server run with replay protection silently disabled"
    );
}

/// (b.2) FAIL-CLOSED at MARK time — when Redis becomes unreachable AFTER a successful connect (a
/// transient outage), `mark` MUST return `Err(ReplayBackendError)` — NEVER `Ok(New)`. The verifier maps
/// that to a 503; the auth path rejects, it never silently accepts a proof while the shared store is
/// down (a fail-OPEN would be a global replay-protection bypass across the fleet).
///
/// Fully self-contained (no external Redis): a tiny fake-Redis TCP server replies `+PONG` to a PING
/// (so `connect`'s eager validation succeeds) but replies with a Redis ERROR (`-ERR ...`) to ANY other
/// command — so the `SET NX PX` that `mark` issues deterministically surfaces as `Err`, regardless of
/// which pooled connection serves it. We assert `mark` fails closed (`Err`), and NEVER `Ok(New)`.
#[test]
fn mark_fails_closed_when_redis_errors() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake-redis listener");
    let addr = listener.local_addr().expect("local addr");

    // Fake-Redis thread: accept each connection and serve RESP commands. The redis client sends a
    // connect-time handshake (a pipelined `CLIENT SETINFO` ×2, then our own `PING`); each must get one
    // reply so the stream stays in sync. The rule: a command whose bytes contain the jti key prefix
    // (`dpop:jti:`) — i.e. the `mark` `SET` — gets a Redis ERROR reply (`-ERR`), which the client
    // surfaces as `Err` → `mark` fails closed. A `PING` gets `+PONG` (the pool's connection check
    // requires it); every other command (SETINFO) gets a benign `+OK`. Replies are emitted ONE PER COMMAND
    // (counted by the RESP array `*` headers), so a pipelined batch doesn't desync the stream.
    let server = std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut sock) = incoming else { break };
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => break, // peer closed / error
                        Ok(n) => {
                            let bytes = &buf[..n];
                            // One reply per command (each starts at a RESP array `*` header), so a
                            // pipelined handshake batch gets the right number of replies (no stream
                            // desync). The jti `SET` gets an ERROR (fail-closed); a `PING` gets
                            // `+PONG`, which the pool's connection check requires; the rest get `+OK`.
                            let mut starts: Vec<usize> = bytes
                                .iter()
                                .enumerate()
                                .filter(|&(_, &b)| b == b'*')
                                .map(|(i, _)| i)
                                .collect();
                            if starts.first() != Some(&0) {
                                starts.insert(0, 0);
                            }
                            let mut ok = true;
                            for (k, &start) in starts.iter().enumerate() {
                                let end = starts.get(k + 1).copied().unwrap_or(bytes.len());
                                let cmd = &bytes[start..end];
                                let reply: &[u8] = if cmd
                                    .windows(b"dpop:jti:".len())
                                    .any(|w| w == b"dpop:jti:")
                                {
                                    b"-ERR simulated redis outage\r\n"
                                } else if cmd.windows(4).any(|w| w.eq_ignore_ascii_case(b"PING")) {
                                    b"+PONG\r\n"
                                } else {
                                    b"+OK\r\n"
                                };
                                if sock.write_all(reply).is_err() {
                                    ok = false;
                                    break;
                                }
                            }
                            if !ok || sock.flush().is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });

    let url = format!("redis://{addr}");
    // connect succeeds (the fake server PONGs the eager PING).
    let store = RedisReplayStore::connect_with_timeout(&url, Duration::from_millis(200))
        .expect("connect should succeed against the PONG-ing fake redis");

    // mark must FAIL CLOSED (Err), never Ok(New): the fake server returns a Redis error to the SET.
    let result = store.mark("any-jti", Duration::from_secs(60));
    assert!(
        result.is_err(),
        "mark MUST fail closed (Err) when Redis errors, never Ok — got {result:?}"
    );
    // Be explicit that it is NOT a silent accept (the fail-open replay-bypass we must never have).
    assert!(
        !matches!(result, Ok(MarkResult::New)),
        "mark must NEVER return Ok(New) on a Redis error (that would be a fail-open replay bypass)"
    );

    drop(store); // closes the job channel; the worker thread ends.
    let _ = server; // detached; ends when the listener drops at test teardown.
}

/// (c) TTL EXPIRY — a `jti` becomes re-markable (`New` again) after its `PX` ttl elapses. Uses a SHORT
/// ttl and sleeps just past it, then asserts the same `jti` is fresh again (mirrors the in-memory
/// store's lazy-expiry; the proof's own `iat` freshness independently rejects a genuinely stale proof).
#[test]
#[ignore = "needs a live Redis (set PSS_IT_REDIS_URL); run with --ignored"]
fn jti_is_remarkable_after_ttl_expiry() {
    let Some(url) = redis_url() else { return };

    let store = RedisReplayStore::connect(&url).expect("connects to Redis");
    let jti = unique_jti("ttl");
    // A short window so the test is quick; comfortably above the op timeout + clock granularity.
    let ttl = Duration::from_millis(300);

    // First mark: fresh.
    assert_eq!(
        store.mark(&jti, ttl).expect("first mark ok"),
        MarkResult::New,
        "first mark of a jti must be New"
    );
    // Within the window: a replay.
    assert_eq!(
        store.mark(&jti, ttl).expect("within-window mark ok"),
        MarkResult::Replay,
        "a re-mark within the PX ttl must be a Replay"
    );

    // Wait until comfortably past the PX ttl so Redis has expired the key.
    std::thread::sleep(ttl + Duration::from_millis(400));

    // After expiry: fresh again (the key is gone; the proof's own iat check guards genuine staleness).
    assert_eq!(
        store.mark(&jti, ttl).expect("post-expiry mark ok"),
        MarkResult::New,
        "after the PX ttl elapses the jti must be re-markable (New)"
    );
}

/// Sanity: a non-positive ttl short-circuits to `New` WITHOUT requiring Redis at all (mirrors the
/// in-memory store; a `PX 0` would be malformed). Runs against a live Redis to confirm the store is
/// otherwise healthy, but the assertion holds regardless of Redis state for a zero ttl.
#[test]
#[ignore = "needs a live Redis (set PSS_IT_REDIS_URL); run with --ignored"]
fn nonpositive_ttl_is_new() {
    let Some(url) = redis_url() else { return };
    let store = RedisReplayStore::connect(&url).expect("connects to Redis");
    let jti = unique_jti("zero");
    assert_eq!(
        store.mark(&jti, Duration::ZERO).expect("zero-ttl mark ok"),
        MarkResult::New,
        "a non-positive ttl must be treated as New without storing"
    );
}

/// (d) `contains()` is a READ-ONLY probe — `false` for an UNSEEN `jti`, `true` AFTER `mark`, and it
/// MUST NOT itself mark: a `contains()` followed by a FRESH `mark()` of the SAME `jti` must see `New`
/// (proving the probe recorded nothing — the [`ReplayStore::contains`] INV-4 read-only contract, and
/// the Redis `EXISTS`-never-`SET` discipline). This is the optimization-hint seam for the held opt-4
/// jti-precheck; it is NOT wired into the auth path yet, so this test is its only coverage of the
/// non-mutating guarantee against a live Redis.
#[test]
#[ignore = "needs a live Redis (set PSS_IT_REDIS_URL); run with --ignored"]
fn contains_is_read_only_false_then_true_after_mark_and_does_not_mark() {
    let Some(url) = redis_url() else { return };
    let store = RedisReplayStore::connect(&url).expect("connects to Redis");

    // --- An UNSEEN jti reads false (EXISTS → 0). ---
    let unseen = unique_jti("contains-unseen");
    assert!(
        !store.contains(&unseen).expect("contains(unseen) ok"),
        "contains() must return false for a jti that has never been marked"
    );

    // --- contains() must NOT itself mark: probing an unseen jti, then a FRESH mark of the SAME jti, ---
    // --- must see New (the probe recorded nothing — a SET would have made the mark a Replay). ---------
    let probe_then_mark = unique_jti("contains-no-mark");
    assert!(
        !store
            .contains(&probe_then_mark)
            .expect("contains(probe) ok"),
        "an unseen jti must read false before any mark"
    );
    // A second probe still false — repeated reads never record state either.
    assert!(
        !store
            .contains(&probe_then_mark)
            .expect("contains(probe-again) ok"),
        "repeated contains() probes must remain non-mutating (still false)"
    );
    assert_eq!(
        store
            .mark(&probe_then_mark, ONE_MINUTE)
            .expect("mark after probe ok"),
        MarkResult::New,
        "a fresh mark after contains() probes MUST be New — contains() must never mark (no SET)"
    );

    // --- AFTER a mark, contains() reads true (EXISTS → 1) for that still-live jti. ---
    let marked = unique_jti("contains-after-mark");
    assert!(
        !store.contains(&marked).expect("contains(pre-mark) ok"),
        "contains() must be false before the mark"
    );
    assert_eq!(
        store.mark(&marked, ONE_MINUTE).expect("mark ok"),
        MarkResult::New,
        "first mark of a jti must be New"
    );
    assert!(
        store.contains(&marked).expect("contains(post-mark) ok"),
        "contains() must return true for a still-live, already-marked jti"
    );
    // And contains() STILL does not mark — the authoritative mark of `marked` is already a Replay, but
    // probing it repeatedly must not change anything (it stays a Replay, not flip to New).
    assert!(
        store
            .contains(&marked)
            .expect("contains(post-mark-again) ok"),
        "a repeated probe of a marked jti stays true (still non-mutating)"
    );
    assert_eq!(
        store.mark(&marked, ONE_MINUTE).expect("re-mark ok"),
        MarkResult::Replay,
        "re-marking an already-marked jti is a Replay, regardless of intervening contains() probes"
    );
}
