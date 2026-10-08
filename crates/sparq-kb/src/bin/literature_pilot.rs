//! `literature-pilot` — the hard-capped, DRY-RUN-FIRST literature ingestion pilot
//! (`sq-tzars.9`, epic `sq-tzars`; design record `research/research-kb-program.md` §2.9).
//! [FABLE-5] 🤖 SPARQ agent — research-KB live ingestion pilot.
//!
//! The iteration protocol itself (PREREG bar → hard caps → dry-run staging → honest
//! append-only metrics → audit → verdict → maintainer-armed live-emit gate) lives in
//! `sparq_kb::literature::pilot` and is enforced there IN CODE; this binary is only the
//! networked/subprocess wiring behind the default-OFF `literature-live` feature.
//!
//! ## Subcommands
//!
//! ```text
//! literature-pilot run --seed <id> [--source openalex|core|fixture] [--fixture <path>]
//!     [--seeds <path>] [--sidecar <path>] [--staging-dir <dir>] [--daily-ledger <path>]
//!     [--extract-cmd "<argv>"] [--extract-tape <path>] [--batch-size N]
//!     [--max-records N] [--max-api-requests N] [--max-invocations N]
//!     [--bar-precision X --bar-min-sample N]
//!     [--live-emit --live-out <dir>]        # SHIPS OFF — see the gate below
//! literature-pilot record-audit  --sidecar <path> --n <N> --precision <X> --auditor <s>
//! literature-pilot record-verdict --sidecar <path> --verdict adopt-topic|iterate|abandon
//!     --notes <s> [--changes <s>]
//! literature-pilot check-gate    --sidecar <path>
//! ```
//!
//! ## The MAINTAINER-ARM boundary (ships OFF)
//!
//! A `run` is a DRY RUN by default and always: it writes ONLY the staging directory and
//! the sidecar. `--live-emit` exists but is triple-gated, fail-closed:
//! 1. the `SPARQ_KB_PILOT_LIVE_EMIT` environment variable must be exactly
//!    `maintainer-armed` (default-unset ⇒ refused — the first live ingestion into the
//!    real KB is the maintainer's action, per the bead's MAINTAINER-ARM label);
//! 2. `pilot::live_emit_allowed` must pass on the run's own sidecar (a recorded audit
//!    clearing the PRE-REGISTERED bar + a passing SHACL check + no non-adopt verdict);
//! 3. the SHACL check requires this binary to be built with the `validate` feature.
//!
//! ## Security posture
//!
//! - `CORE_API_KEY` (CORE source) travels only in the `Authorization` header via the
//!   existing `connector_core` transport; never in a URL, log line, or error.
//! - `OPENALEX_API_KEY` (optional premium key) and `OPENALEX_MAILTO` (polite pool) are
//!   appended to the OpenAlex request URL per that API's design; when a key is present
//!   the URL is NEVER printed (errors reference the endpoint constant only). OpenAlex
//!   works keyless — the default is the anonymous/polite pool.
//! - The extractor command (`SPARQ_KB_EXTRACT_CMD` / `--extract-cmd`) is default-unset;
//!   without it a live extraction cannot run (the seam is inert by construction).
//! - The Semantic Scholar slot is REGISTERED but GATED (#1139: no key) — selecting
//!   `--source s2` errors with the gating note; the `SourceStub` boundary keeps it
//!   pluggable at zero architectural cost.
//!
//! Work-box timings are non-canonical; this binary records decision metrics
//! (grounding / conformance / quarantine / audited precision) only.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use sparq_kb::literature::connector_core::{
    fetch_paginated, percent_encode_query, HttpResponse, RetryPolicy, Transport, UreqTransport,
    CORE_SEARCH_WORKS_URL,
};
use sparq_kb::literature::extract::{Extractor, RecordedExtractor};
use sparq_kb::literature::extract_live::{CommandRunner, LiveExtractor};
use sparq_kb::literature::pilot::{
    self, combine_records, normalise_openalex_records, parse_seed_registry, records_from_stubs,
    today_utc, CapLedger, DailyLedger, DryRunInputs, FetchStats, HardCaps, PilotConfig, PilotRun,
    PreregBar, SeedSpec, SidecarFile,
};

/// The OpenAlex works-search endpoint (keyless polite pool by default).
const OPENALEX_WORKS_URL: &str = "https://api.openalex.org/works";

/// The maintainer-arm acknowledgement env var + required value for `--live-emit`
/// (ships OFF: default-unset ⇒ every live emit refuses).
const LIVE_EMIT_ENV: &str = "SPARQ_KB_PILOT_LIVE_EMIT";
const LIVE_EMIT_ACK: &str = "maintainer-armed";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = dispatch(&args) {
        eprintln!("literature-pilot: {}", e);
        std::process::exit(1);
    }
}

fn dispatch(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..]),
        Some("record-audit") => cmd_record_audit(&args[1..]),
        Some("record-verdict") => cmd_record_verdict(&args[1..]),
        Some("check-gate") => cmd_check_gate(&args[1..]),
        _ => Err("usage: literature-pilot run|record-audit|record-verdict|check-gate … \
                  (see the module docs)"
            .to_string()),
    }
}

/// Minimal flag reader: `--key value` pairs plus bare `--flag` booleans.
struct Flags<'a> {
    args: &'a [String],
}

impl<'a> Flags<'a> {
    fn get(&self, key: &str) -> Option<&'a str> {
        self.args
            .iter()
            .position(|a| a == key)
            .and_then(|i| self.args.get(i + 1))
            .map(String::as_str)
    }
    fn has(&self, key: &str) -> bool {
        self.args.iter().any(|a| a == key)
    }
    fn get_usize(&self, key: &str) -> Result<Option<usize>, String> {
        self.get(key)
            .map(|v| v.parse().map_err(|e| format!("{}: {}", key, e)))
            .transpose()
    }
    fn get_f64(&self, key: &str) -> Result<Option<f64>, String> {
        self.get(key)
            .map(|v| v.parse().map_err(|e| format!("{}: {}", key, e)))
            .transpose()
    }
}

fn cmd_record_audit(args: &[String]) -> Result<(), String> {
    let f = Flags { args };
    let sidecar = SidecarFile::new(PathBuf::from(
        f.get("--sidecar").ok_or("record-audit: --sidecar required")?,
    ));
    let n = f
        .get_usize("--n")?
        .ok_or("record-audit: --n <sample size> required")?;
    let precision = f
        .get_f64("--precision")?
        .ok_or("record-audit: --precision required")?;
    let auditor = f.get("--auditor").ok_or("record-audit: --auditor required")?;
    let passed = pilot::record_audit_at(&sidecar, n, precision, auditor)?;
    println!(
        "audit recorded verbatim: n={} precision={} passed_bar={}",
        n, precision, passed
    );
    Ok(())
}

fn cmd_record_verdict(args: &[String]) -> Result<(), String> {
    let f = Flags { args };
    let sidecar = SidecarFile::new(PathBuf::from(
        f.get("--sidecar").ok_or("record-verdict: --sidecar required")?,
    ));
    let verdict = f.get("--verdict").ok_or("record-verdict: --verdict required")?;
    let notes = f.get("--notes").ok_or("record-verdict: --notes required")?;
    let changes = f.get("--changes").unwrap_or("none recorded");
    pilot::record_verdict_at(&sidecar, verdict, notes, changes)?;
    println!("verdict recorded: {}", verdict);
    Ok(())
}

fn cmd_check_gate(args: &[String]) -> Result<(), String> {
    let f = Flags { args };
    let path = f.get("--sidecar").ok_or("check-gate: --sidecar required")?;
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    match pilot::live_emit_allowed(&raw) {
        Ok(()) => {
            println!("live-emit gate: WOULD PASS (the emit itself stays maintainer-armed)");
            Ok(())
        }
        Err(e) => {
            println!("live-emit gate: REFUSED — {}", e);
            Ok(())
        }
    }
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    let f = Flags { args };
    let seed_id = f.get("--seed").ok_or("run: --seed <id> required")?;
    let seeds_path = f
        .get("--seeds")
        .unwrap_or("crates/sparq-kb/ingest/literature-seeds.toml");
    let seeds_raw = std::fs::read_to_string(seeds_path)
        .map_err(|e| format!("seed registry {}: {}", seeds_path, e))?;
    let seeds = parse_seed_registry(&seeds_raw)?;
    let seed: SeedSpec = seeds
        .iter()
        .find(|s| s.id == seed_id)
        .cloned()
        .ok_or_else(|| format!("run: seed {:?} not in the registry", seed_id))?;

    let source = f.get("--source").unwrap_or("openalex").to_string();
    let mut caps = HardCaps::default();
    if let Some(n) = f.get_usize("--max-records")? {
        caps.max_records = n;
    }
    if let Some(n) = f.get_usize("--max-api-requests")? {
        caps.max_api_requests = n;
    }
    if let Some(n) = f.get_usize("--max-invocations")? {
        caps.max_subagent_invocations = n;
    }
    // The seed's own budget also caps the run (the tighter bound wins).
    caps.max_records = caps.max_records.min(seed.max_records);

    let mut bar = PreregBar::default();
    let mut bar_overridden = false;
    if let Some(p) = f.get_f64("--bar-precision")? {
        bar.min_precision = p;
        bar_overridden = true;
    }
    if let Some(n) = f.get_usize("--bar-min-sample")? {
        bar.min_sample = n;
        bar_overridden = true;
    }

    let created_at = pilot::now_utc_iso();
    let run_id = format!(
        "{}-{}",
        seed.id,
        created_at
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
    );
    let staging_root = PathBuf::from(
        f.get("--staging-dir")
            .unwrap_or("target/literature-pilot/staging"),
    );
    let staging = staging_root.join(&run_id);
    let sidecar_path = f
        .get("--sidecar")
        .map(PathBuf::from)
        .unwrap_or_else(|| staging.join("sidecar.jsonl"));
    let daily_ledger = DailyLedger::new(
        f.get("--daily-ledger")
            .map(PathBuf::from)
            .unwrap_or_else(|| staging_root.join("daily-ledger.json")),
    );

    // PREREG FIRST — before any fetch or extraction (the bar is now committed to the
    // sidecar; everything below is reachable only through the pre-registered run).
    let cfg = PilotConfig {
        run_id: run_id.clone(),
        seed: seed.clone(),
        source: source.clone(),
        caps,
        bar,
        bar_overridden,
        dry_run: true,
        created_at,
    };
    let run = PilotRun::preregister(cfg, sidecar_path.clone())?;
    println!("preregistered run {} (sidecar: {})", run_id, sidecar_path.display());

    // Daily API budget (fail-stop BEFORE any request): the per-run request cap is
    // narrowed to what is left of today's budget.
    let today = today_utc();
    let used_today = daily_ledger.used_today(&today)?;
    let remaining_today = caps.max_api_requests_per_day.saturating_sub(used_today);
    if remaining_today == 0 && source != "fixture" {
        return Err(format!(
            "cap fail-stop: the daily API-request budget of {} is exhausted",
            caps.max_api_requests_per_day
        ));
    }
    let mut effective_caps = caps;
    effective_caps.max_api_requests = caps.max_api_requests.min(remaining_today);
    let ledger = RefCell::new(CapLedger::new(effective_caps));

    // FETCH (charged per-request through the ledger — fail-stop).
    let (batch_json, fetch, endpoint) = fetch_batch(&f, &source, &seed, &ledger)?;
    // Charge the actual requests made against the day budget (fail-stop already applied
    // per-request via the narrowed per-run cap, so this cannot exceed the daily cap).
    let requests_made = ledger.borrow().api_requests_used();
    if requests_made > 0 {
        daily_ledger.charge(&today, requests_made, caps.max_api_requests_per_day)?;
    }

    // EXTRACT + GROUND + TIER + SHACL + STAGING + METRICS (all inside the lib protocol).
    let batch_size = f.get_usize("--batch-size")?.unwrap_or(6).max(1);
    let inputs = DryRunInputs {
        batch_json: &batch_json,
        fetch,
        batch_size,
        staging_dir: Some(&staging),
        generated_at_time: None,
        source_endpoint: endpoint,
    };
    let metrics = if let Some(tape) = f.get("--extract-tape") {
        let raw = std::fs::read_to_string(tape).map_err(|e| format!("{}: {}", tape, e))?;
        let ex = RecordedExtractor::from_tape(&raw)?;
        execute(&run, &inputs, &ex, &ledger)?
    } else {
        let runner = match f.get("--extract-cmd") {
            Some(cmd) => CommandRunner::from_command(cmd)?,
            None => CommandRunner::from_env()?,
        };
        let ex = LiveExtractor::new(runner);
        execute(&run, &inputs, &ex, &ledger)?
    };

    println!(
        "dry run complete: fetched={} boundary_rejects={} candidates={} grounded={} \
         (rate {:.4}) quarantined={} \
         shacl_checked={} shacl_conforms={:?} machine={} restricted={} sample={} staging={}",
        metrics.fetched_records,
        metrics.boundary_rejects,
        metrics.candidates_total,
        metrics.grounded,
        metrics.grounding_rate,
        metrics.quarantined,
        metrics.shacl_checked,
        metrics.shacl_conforms,
        metrics.machine_tier_findings,
        metrics.license_restricted_findings,
        metrics.audit_sample.len(),
        staging.display(),
    );
    println!(
        "next: audit the sample (staging audit-sample.json), then `record-audit`, then \
         `record-verdict`. No KB write happened (dry-run default)."
    );

    // The MAINTAINER-ARM live-emit boundary — ships OFF (default-unset env ⇒ refuse).
    if f.has("--live-emit") {
        let ack = std::env::var(LIVE_EMIT_ENV).unwrap_or_default();
        if ack != LIVE_EMIT_ACK {
            return Err(format!(
                "--live-emit REFUSED (fail-closed): the first live ingestion into the real KB \
                 is maintainer-armed. Set {}={} only under the maintainer's explicit arm.",
                LIVE_EMIT_ENV, LIVE_EMIT_ACK
            ));
        }
        // Even with the ack, the gate demands a recorded PASSING audit on the sidecar —
        // which a just-finished run cannot have yet. The flag is therefore structurally
        // OFF for single-invocation use; a maintainer replays a previously-audited run's
        // artifacts via the gate. This is deliberate.
        let raw = std::fs::read_to_string(run.sidecar().path()).map_err(|e| e.to_string())?;
        pilot::live_emit_allowed(&raw)?;
        return Err(
            "live emit path is gated end-to-end and intentionally not wired to a KB writer \
             in this harness PR (sq-tzars.9 ships the flag OFF; the maintainer arms the \
             first live ingestion)"
                .to_string(),
        );
    }
    Ok(())
}

/// Run the dry pilot with the SHACL gate when built with `validate` (a run whose SHACL
/// leg is absent records `shacl_checked=false` and can never pass the live-emit gate).
fn execute<E: Extractor>(
    run: &PilotRun,
    inputs: &DryRunInputs<'_>,
    extractor: &E,
    ledger: &RefCell<CapLedger>,
) -> Result<pilot::RunMetrics, String> {
    let mut l = ledger.borrow_mut();
    #[cfg(feature = "validate")]
    {
        let gate = |ttl: &str| -> Result<bool, String> {
            let base = "https://sparq.dev/ns/pkg/example#";
            let data = sparq_kb::validate::graph_from_turtle_docs(
                &[sparq_kb::PKG_ONTOLOGY, ttl],
                base,
            )?;
            let shapes = sparq_kb::validate::graph_from_turtle_docs(
                &[
                    sparq_kb::PKG_SHAPES,
                    sparq_kb::literature::LITERATURE_SHAPES,
                ],
                base,
            )?;
            let report = sparq_shacl::validate(&data, &shapes);
            Ok(report.conforms_violations_only())
        };
        run.execute_dry(inputs, extractor, &mut l, Some(&gate))
    }
    #[cfg(not(feature = "validate"))]
    {
        run.execute_dry(inputs, extractor, &mut l, None)
    }
}

/// A `Transport` that charges every request to the cap ledger BEFORE performing it
/// (fail-stop: a request past the cap is never made).
struct LedgerTransport<'a, T: Transport> {
    inner: T,
    ledger: &'a RefCell<CapLedger>,
}

impl<T: Transport> Transport for LedgerTransport<'_, T> {
    fn get(&self, url: &str) -> Result<HttpResponse, String> {
        self.ledger.borrow_mut().try_charge_api_request()?;
        self.inner.get(url)
    }
}

/// A keyless blocking transport for OpenAlex (no `Authorization` header; rustls).
struct PlainTransport {
    timeout: Duration,
}

impl Transport for PlainTransport {
    fn get(&self, url: &str) -> Result<HttpResponse, String> {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .http_status_as_error(false)
            .user_agent(concat!("sparq-kb/", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let mut resp = agent
            .get(url)
            .header("Accept", "application/json")
            .call()
            // NEVER interpolate the URL: it may carry the optional OpenAlex api_key.
            .map_err(|e| format!("OpenAlex: request failed: {}", e))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        let body = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| format!("OpenAlex: reading response body: {}", e))?;
        Ok(HttpResponse {
            status,
            retry_after,
            body,
        })
    }
}

/// Fetch one batch from the selected source, returning the combined OpenAlex-shaped
/// batch JSON + fetch stats + the provenance endpoint IRI (never carrying a key).
fn fetch_batch(
    f: &Flags<'_>,
    source: &str,
    seed: &SeedSpec,
    ledger: &RefCell<CapLedger>,
) -> Result<(String, FetchStats, &'static str), String> {
    let max_records = ledger.borrow().caps().max_records;
    match source {
        "fixture" => {
            let path = f
                .get("--fixture")
                .ok_or("run: --source fixture requires --fixture <path>")?;
            let batch =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
            let records = normalise_openalex_records(&batch)?;
            let stats = FetchStats {
                fetched_records: records.len(),
                api_requests: 0,
                pages_fetched: 0,
                complete: true,
                records_capped: false,
            };
            Ok((
                combine_records(&records),
                stats,
                "https://sparq.dev/ns/pkg/example#fixture",
            ))
        }
        "openalex" => fetch_openalex(seed, max_records, ledger),
        "core" => fetch_core(seed, max_records, ledger),
        "s2" | "semantic-scholar" => Err(
            "the Semantic Scholar slot is PLUGGABLE but GATED (#1139: no key provisioned; \
             S2 data additionally carries a no-redistribution licensing constraint) — use \
             --source openalex or --source core"
                .to_string(),
        ),
        other => Err(format!("run: unknown --source {:?}", other)),
    }
}

/// Paged OpenAlex fetch (keyless by default; optional `OPENALEX_MAILTO` polite-pool tag
/// and `OPENALEX_API_KEY` premium key — both appended as query parameters per the
/// OpenAlex API design, and the URL is never printed). Fail-stop request charging;
/// completeness is fail-closed (only an exhausted result set is `complete`).
fn fetch_openalex(
    seed: &SeedSpec,
    max_records: usize,
    ledger: &RefCell<CapLedger>,
) -> Result<(String, FetchStats, &'static str), String> {
    let per_page = max_records.clamp(1, 25);
    let policy = RetryPolicy {
        page_size: per_page,
        ..RetryPolicy::default()
    };
    let transport = LedgerTransport {
        inner: PlainTransport {
            timeout: Duration::from_secs(30),
        },
        ledger,
    };
    let mailto = std::env::var("OPENALEX_MAILTO").ok().filter(|s| !s.is_empty());
    let api_key = std::env::var("OPENALEX_API_KEY").ok().filter(|s| !s.is_empty());

    let mut records = Vec::new();
    let mut stats = FetchStats::default();
    let mut page = 1usize;
    loop {
        if records.len() >= max_records {
            stats.records_capped = true;
            break;
        }
        let mut url = format!(
            "{}?search={}&per-page={}&page={}",
            OPENALEX_WORKS_URL,
            percent_encode_query(&seed.query),
            per_page,
            page
        );
        if let Some(m) = &mailto {
            url.push_str("&mailto=");
            url.push_str(&percent_encode_query(m));
        }
        if let Some(k) = &api_key {
            url.push_str("&api_key=");
            url.push_str(&percent_encode_query(k));
        }
        let resp = get_with_retry(&transport, &url, &policy)?;
        let page_records = normalise_openalex_records(&resp.body)?;
        let got = page_records.len();
        let total = serde_json::from_str::<serde_json::Value>(&resp.body)
            .ok()
            .and_then(|v| v.get("meta").and_then(|m| m.get("count")).and_then(|c| c.as_u64()))
            .map(|n| n as usize);
        records.extend(page_records);
        stats.pages_fetched += 1;
        if got == 0 || got < per_page {
            stats.complete = true;
            break;
        }
        if let Some(t) = total {
            if page * per_page >= t {
                stats.complete = true;
                break;
            }
        }
        page += 1;
    }
    if records.len() > max_records {
        records.truncate(max_records);
        stats.records_capped = true;
        stats.complete = false;
    }
    stats.fetched_records = records.len();
    stats.api_requests = ledger.borrow().api_requests_used();
    Ok((combine_records(&records), stats, OPENALEX_WORKS_URL))
}

/// One GET with the connector's retry discipline (`Retry-After` honoured, bounded
/// exponential backoff), each attempt charged fail-stop through the ledger transport.
fn get_with_retry<T: Transport>(
    transport: &T,
    url: &str,
    policy: &RetryPolicy,
) -> Result<HttpResponse, String> {
    use sparq_kb::literature::connector_core::{backoff_delay, is_retryable_status};
    let mut attempt = 0u32;
    loop {
        let resp = transport.get(url)?;
        if is_retryable_status(resp.status) {
            if attempt >= policy.max_retries {
                return Err(format!(
                    "OpenAlex: gave up after {} retries; last HTTP status {}",
                    policy.max_retries, resp.status
                ));
            }
            std::thread::sleep(backoff_delay(attempt, resp.retry_after, policy));
            attempt += 1;
            continue;
        }
        if resp.status >= 400 {
            return Err(format!("OpenAlex: non-retryable HTTP status {}", resp.status));
        }
        return Ok(resp);
    }
}

/// Paged CORE v3 fetch through the existing `connector_core` machinery (key in the
/// `Authorization` header only), with every request charged through the ledger.
fn fetch_core(
    seed: &SeedSpec,
    max_records: usize,
    ledger: &RefCell<CapLedger>,
) -> Result<(String, FetchStats, &'static str), String> {
    let api_key = std::env::var("CORE_API_KEY").map_err(|_| {
        "CORE: CORE_API_KEY is not set (load it locally; never committed or logged)".to_string()
    })?;
    if api_key.trim().is_empty() {
        return Err("CORE: CORE_API_KEY is set but empty".to_string());
    }
    let page_size = max_records.clamp(1, 25);
    let policy = RetryPolicy {
        page_size,
        max_pages: max_records.div_ceil(page_size),
        ..RetryPolicy::default()
    };
    let transport = LedgerTransport {
        inner: UreqTransport::new(api_key, Duration::from_secs(30)),
        ledger,
    };
    let result = fetch_paginated(
        &transport,
        CORE_SEARCH_WORKS_URL,
        &seed.query,
        &policy,
        &mut |d| std::thread::sleep(d),
    )?;
    let mut stubs = result.stubs;
    let mut records_capped = false;
    if stubs.len() > max_records {
        stubs.truncate(max_records);
        records_capped = true;
    }
    let complete = result.pages_fetched < policy.max_pages && !records_capped;
    let stats = FetchStats {
        fetched_records: stubs.len(),
        api_requests: ledger.borrow().api_requests_used(),
        pages_fetched: result.pages_fetched,
        complete,
        records_capped,
    };
    Ok((
        combine_records(&records_from_stubs(&stubs)),
        stats,
        CORE_SEARCH_WORKS_URL,
    ))
}
