//! The **hard-capped, dry-run-first literature pilot loop** — the iteration protocol of
//! `sq-tzars.9` (epic `sq-tzars`, design records `research/research-kb-program.md` §2.9 +
//! `research/provenance-driven-genai-kb.md` §4/§6.4), ENFORCED IN CODE, not in prose.
//! [FABLE-5] 🤖 SPARQ agent — research-KB live ingestion pilot.
//!
//! ## The protocol (each rule has a structural enforcement point here)
//!
//! 1. **PREREG** — the audit bar is written to the run sidecar *before* any extraction.
//!    `PilotRun` is only constructible via `PilotRun::preregister`, which appends the
//!    `prereg` record first; every extraction entry point is a method on `PilotRun`, so a
//!    run that has not pre-registered its bar *cannot* extract (type-state, not convention).
//!    The default bar (`PreregBar::default`: audited precision ≥ 0.8 on a ≥ 20-finding
//!    uniform sample) is a **pre-registered threshold choice, NOT a measured claim**; a
//!    maintainer-steered override is recorded verbatim in the sidecar (`overridden: true`).
//! 2. **DRY-RUN DEFAULT** — a pilot run writes ONLY to its staging directory. No KB
//!    mutation and no dump-facing artifact can be produced until `live_emit_allowed`
//!    passes: a recorded audit (a `stage: "audit"` sidecar record) whose sample size and
//!    audited precision clear the *pre-registered* bar, plus a SHACL-conformant metrics
//!    record. `live_emit` refuses (fail-closed) otherwise.
//! 3. **HARD CAPS** — `CapLedger` enforces per-run ceilings (records, API requests,
//!    sub-agent invocations, prompt bytes as the cost proxy) and `DailyLedger` a
//!    per-day API-request ceiling. All are **fail-stop**: exceeding a cap is an `Err`
//!    that aborts the stage — never a silent best-effort continuation. Batched extraction
//!    pre-checks its whole budget BEFORE the first invocation.
//! 4. **HONEST METRICS** — the sidecar is an append-only JSONL chain (`SidecarFile`):
//!    each line carries `seq` + the FNV-1a hash of the previous raw line, so any
//!    retro-edit breaks verification and further appends refuse. Metrics (grounding rate,
//!    SHACL conformance, quarantine counts + reasons, extractor boundary-reject counts +
//!    reasons, audited precision) are recorded
//!    verbatim. The committed sidecar carries **no abstract-derived text** (fail-closed
//!    licensing: justifications stay in the LOCAL staging area only).
//! 5. **VERDICT** — one terminal `adopt-topic | iterate | abandon` record per run;
//!    `adopt-topic` is refused unless the recorded audit passed the pre-registered bar.
//!
//! Confidence values everywhere in this pipeline are **UNCALIBRATED** (master record
//! §6.4; the reliability-diagram harness is rung C, bead `sq-2489d.10`): the sidecar
//! carries `UNCALIBRATED_NOTICE` structurally so no consumer can miss it.
//!
//! The networked / subprocess wiring (OpenAlex + CORE fetch, the live extractor command)
//! lives in the `literature-pilot` binary behind the default-OFF `literature-live`
//! feature; everything here is pure and exercised over fixtures with ZERO network and
//! ZERO live-model calls. Work-box timings are non-canonical and are deliberately NOT
//! recorded as performance claims.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::connector::{parse_openalex_batch, SourceStub};
use super::extract::{BoundaryReject, CandidateFinding, Extractor};
use super::extract_live::build_batch_prompt;
use super::pipeline::{
    self, current_generated_at_time, BatchCompleteness, TieredOutput, MACHINE_AGENT_IRI,
};

/// The structural UNCALIBRATED disclaimer stamped into every sidecar `prereg` + `metrics`
/// record (§6.4 rung B, the same mechanism `sq-2489d.9` gives `sparq-nlq`; this crate
/// carries its own byte-stable copy because the two crates are independent). Confidence
/// values in this pipeline are machine-capped estimates with NO reliability measurement
/// behind them; rung C (`sq-2489d.10`) builds the calibration harness.
pub const UNCALIBRATED_NOTICE: &str = "confidence values are UNCALIBRATED machine-capped estimates — no reliability measurement exists yet (master record §6.4; rung C = sq-2489d.10)";

/// The sidecar schema identifier (append-only versioning: a breaking change bumps the
/// suffix, existing sidecars are never rewritten).
pub const SIDECAR_SCHEMA: &str = "sparq-kb/literature-pilot-sidecar/v1";

/// Default pre-registered audit-bar minimum precision (a threshold CHOICE, not a
/// measured claim — see the design record §2.9).
pub const DEFAULT_BAR_MIN_PRECISION: f64 = 0.8;
/// Default pre-registered audit-bar minimum uniform-sample size.
pub const DEFAULT_BAR_MIN_SAMPLE: usize = 20;

// --------------------------------------------------------------------------------------
// Pre-registered bar + hard caps
// --------------------------------------------------------------------------------------

/// The pre-registered audit bar: the run passes only if a recorded audit of a uniform
/// sample of ≥ `min_sample` findings reports precision ≥ `min_precision`. Written to the
/// sidecar BEFORE extraction; the gate reads the bar back from the *sidecar*, so a
/// post-hoc caller cannot substitute a softer bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreregBar {
    /// Minimum audited precision in `[0, 1]`.
    pub min_precision: f64,
    /// Minimum uniform-sample size the audit must cover.
    pub min_sample: usize,
}

impl Default for PreregBar {
    fn default() -> Self {
        Self {
            min_precision: DEFAULT_BAR_MIN_PRECISION,
            min_sample: DEFAULT_BAR_MIN_SAMPLE,
        }
    }
}

/// Per-run hard caps, enforced **fail-stop** by [`CapLedger`]. Defaults are conservative
/// discipline knobs (not tuned numbers); every value is recorded verbatim in the sidecar
/// `prereg` record so a run can never claim a cap it did not register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardCaps {
    /// Max records (papers) ingested per run.
    pub max_records: usize,
    /// Max literature-API HTTP requests per run (incl. retries).
    pub max_api_requests: usize,
    /// Max literature-API HTTP requests per UTC day (all runs; see [`DailyLedger`]).
    pub max_api_requests_per_day: usize,
    /// Max sub-agent (extractor model) invocations per run.
    pub max_subagent_invocations: usize,
    /// Max prompt bytes per single sub-agent invocation (cost ceiling, per call).
    pub max_prompt_bytes_per_invocation: usize,
    /// Max total prompt bytes per run (the run-level cost ceiling proxy).
    pub max_prompt_bytes_total: usize,
}

impl Default for HardCaps {
    fn default() -> Self {
        Self {
            max_records: 40,
            max_api_requests: 12,
            max_api_requests_per_day: 200,
            max_subagent_invocations: 8,
            max_prompt_bytes_per_invocation: 131_072,
            max_prompt_bytes_total: 1_048_576,
        }
    }
}

/// The per-run cap ledger. Every `try_charge_*` is FAIL-STOP: it refuses the charge (and
/// the caller must abort the stage) rather than partially exceeding a cap.
#[derive(Debug, Clone)]
pub struct CapLedger {
    caps: HardCaps,
    records: usize,
    api_requests: usize,
    invocations: usize,
    prompt_bytes: usize,
}

impl CapLedger {
    /// A fresh ledger over the given caps.
    pub fn new(caps: HardCaps) -> Self {
        Self {
            caps,
            records: 0,
            api_requests: 0,
            invocations: 0,
            prompt_bytes: 0,
        }
    }

    /// The registered caps.
    pub fn caps(&self) -> &HardCaps {
        &self.caps
    }

    /// Records charged so far.
    pub fn records_used(&self) -> usize {
        self.records
    }

    /// API requests charged so far.
    pub fn api_requests_used(&self) -> usize {
        self.api_requests
    }

    /// Sub-agent invocations charged so far.
    pub fn invocations_used(&self) -> usize {
        self.invocations
    }

    /// Total prompt bytes charged so far.
    pub fn prompt_bytes_used(&self) -> usize {
        self.prompt_bytes
    }

    /// Sub-agent invocations still available.
    pub fn remaining_invocations(&self) -> usize {
        self.caps.max_subagent_invocations.saturating_sub(self.invocations)
    }

    /// Charge `n` ingested records; fail-stop if the run's record cap would be exceeded.
    pub fn try_charge_records(&mut self, n: usize) -> Result<(), String> {
        if self.records + n > self.caps.max_records {
            return Err(format!(
                "cap fail-stop: {} records would exceed the per-run record cap of {} ({} already charged)",
                n, self.caps.max_records, self.records
            ));
        }
        self.records += n;
        Ok(())
    }

    /// Charge one API request; fail-stop at the per-run request cap.
    pub fn try_charge_api_request(&mut self) -> Result<(), String> {
        if self.api_requests + 1 > self.caps.max_api_requests {
            return Err(format!(
                "cap fail-stop: per-run API request cap of {} reached",
                self.caps.max_api_requests
            ));
        }
        self.api_requests += 1;
        Ok(())
    }

    /// Charge one sub-agent invocation carrying a prompt of `prompt_len` bytes; fail-stop
    /// at the invocation cap, the per-invocation prompt ceiling, or the run cost ceiling.
    pub fn try_charge_invocation(&mut self, prompt_len: usize) -> Result<(), String> {
        if self.invocations + 1 > self.caps.max_subagent_invocations {
            return Err(format!(
                "cap fail-stop: per-run sub-agent invocation cap of {} reached",
                self.caps.max_subagent_invocations
            ));
        }
        if prompt_len > self.caps.max_prompt_bytes_per_invocation {
            return Err(format!(
                "cap fail-stop: a {}-byte prompt exceeds the per-invocation ceiling of {} bytes",
                prompt_len, self.caps.max_prompt_bytes_per_invocation
            ));
        }
        if self.prompt_bytes + prompt_len > self.caps.max_prompt_bytes_total {
            return Err(format!(
                "cap fail-stop: total prompt bytes would exceed the run cost ceiling of {} bytes",
                self.caps.max_prompt_bytes_total
            ));
        }
        self.invocations += 1;
        self.prompt_bytes += prompt_len;
        Ok(())
    }
}

/// The per-UTC-day API-request ledger (`requests/day` cap), persisted as a tiny JSON file
/// (`{"date": "YYYY-MM-DD", "api_requests": N}`). [`DailyLedger::charge`] is FAIL-STOP:
/// it refuses a charge that would exceed the daily cap. A new date resets the counter.
#[derive(Debug, Clone)]
pub struct DailyLedger {
    path: PathBuf,
}

impl DailyLedger {
    /// A ledger persisted at `path` (created on first charge).
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read(&self, today: &str) -> Result<usize, String> {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(r) => r,
            Err(_) => return Ok(0), // absent file = nothing used
        };
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("daily ledger {} is corrupt: {}", self.path.display(), e))?;
        if v.get("date").and_then(Value::as_str) == Some(today) {
            Ok(v.get("api_requests").and_then(Value::as_u64).unwrap_or(0) as usize)
        } else {
            Ok(0) // a different (older) date resets the day budget
        }
    }

    /// Requests already used today.
    pub fn used_today(&self, today: &str) -> Result<usize, String> {
        self.read(today)
    }

    /// Charge `n` requests against `cap` for `today` (UTC `YYYY-MM-DD`); fail-stop if the
    /// day budget would be exceeded (the file is NOT updated on refusal). Returns the new
    /// used-today total.
    pub fn charge(&self, today: &str, n: usize, cap: usize) -> Result<usize, String> {
        let used = self.read(today)?;
        if used + n > cap {
            return Err(format!(
                "cap fail-stop: {} requests would exceed the daily API-request cap of {} ({} already used on {})",
                n, cap, used, today
            ));
        }
        let new_used = used + n;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("daily ledger dir: {}", e))?;
        }
        std::fs::write(
            &self.path,
            json!({ "date": today, "api_requests": new_used }).to_string(),
        )
        .map_err(|e| format!("daily ledger write: {}", e))?;
        Ok(new_used)
    }
}

// --------------------------------------------------------------------------------------
// The append-only sidecar chain
// --------------------------------------------------------------------------------------

/// FNV-1a 64-bit hash (dep-free). Used for the sidecar's append-integrity chain — this is
/// **tamper-evident against accidental retro-editing, NOT cryptographic**; the durable
/// audit trail is the git history of the committed sidecar.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Verify an append-only sidecar chain: every line is a JSON object carrying `seq`
/// (0-based, contiguous) and `prev` (`"genesis"` for the first line, else the FNV-1a hash
/// of the previous RAW line, hex). Returns the parsed records; any retro-edited,
/// reordered, or deleted line breaks the chain and is an `Err`.
pub fn verify_chain(raw: &str) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    let mut prev_line: Option<&str> = None;
    for (i, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(format!("sidecar chain: blank line at {}", i));
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| format!("sidecar chain: line {} is not JSON: {}", i, e))?;
        let seq = v.get("seq").and_then(Value::as_u64);
        if seq != Some(i as u64) {
            return Err(format!(
                "sidecar chain: line {} carries seq {:?}, expected {} (append-only order broken)",
                i, seq, i
            ));
        }
        let expected_prev = match prev_line {
            None => "genesis".to_string(),
            Some(p) => format!("{:016x}", fnv1a64(p.as_bytes())),
        };
        let prev = v.get("prev").and_then(Value::as_str).unwrap_or("");
        if prev != expected_prev {
            return Err(format!(
                "sidecar chain: line {} prev-hash mismatch — the sidecar was retro-edited \
                 (append-only invariant violated)",
                i
            ));
        }
        records.push(v);
        prev_line = Some(line);
    }
    Ok(records)
}

/// The append-only run sidecar. Records append with `seq` + `prev` chain fields; the file
/// is verified before EVERY append, so a retro-edited sidecar refuses further appends.
/// Stage ordering is enforced: `prereg` first (exactly once), then `metrics` (at most
/// once), then `audit` (repeatable), then a terminal `verdict` (at most once; nothing
/// appends after it).
#[derive(Debug, Clone)]
pub struct SidecarFile {
    path: PathBuf,
}

impl SidecarFile {
    /// A sidecar at `path` (created by the first append).
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The sidecar path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read + verify the chain (empty file/absent file = empty chain).
    pub fn chain(&self) -> Result<Vec<Value>, String> {
        let raw = std::fs::read_to_string(&self.path).unwrap_or_default();
        verify_chain(&raw)
    }

    /// Append one record (must be an object with a `"stage"` field). Verifies the
    /// existing chain, enforces the stage-order rules, stamps `seq`/`prev`, and appends.
    pub fn append(&self, mut record: Value) -> Result<(), String> {
        let stage = record
            .get("stage")
            .and_then(Value::as_str)
            .ok_or_else(|| "sidecar append: record has no `stage` field".to_string())?
            .to_string();
        let raw = std::fs::read_to_string(&self.path).unwrap_or_default();
        let existing = verify_chain(&raw)?;
        enforce_stage_order(&existing, &stage, &record)?;

        let seq = existing.len() as u64;
        let prev = match raw.lines().last() {
            None => "genesis".to_string(),
            Some(p) => format!("{:016x}", fnv1a64(p.as_bytes())),
        };
        let obj = record
            .as_object_mut()
            .ok_or_else(|| "sidecar append: record must be a JSON object".to_string())?;
        obj.insert("seq".to_string(), json!(seq));
        obj.insert("prev".to_string(), json!(prev));

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("sidecar dir: {}", e))?;
        }
        let mut out = raw;
        out.push_str(&record.to_string());
        out.push('\n');
        std::fs::write(&self.path, out).map_err(|e| format!("sidecar write: {}", e))
    }
}

/// The stage-order rules (fail-closed). `adopt-topic` verdicts additionally require the
/// LAST recorded audit to have passed the pre-registered bar.
fn enforce_stage_order(existing: &[Value], stage: &str, record: &Value) -> Result<(), String> {
    let stage_of = |v: &Value| v.get("stage").and_then(Value::as_str).unwrap_or("").to_string();
    if existing.iter().any(|v| stage_of(v) == "verdict") {
        return Err("sidecar: the verdict is terminal — nothing appends after it".to_string());
    }
    let has_prereg = existing.iter().any(|v| stage_of(v) == "prereg");
    let has_metrics = existing.iter().any(|v| stage_of(v) == "metrics");
    match stage {
        "prereg" => {
            if !existing.is_empty() {
                return Err("sidecar: prereg must be the FIRST record (exactly once)".to_string());
            }
            Ok(())
        }
        "metrics" => {
            if !has_prereg {
                return Err(
                    "sidecar: metrics before prereg — the bar must be pre-registered first"
                        .to_string(),
                );
            }
            if has_metrics {
                return Err("sidecar: a run records metrics exactly once".to_string());
            }
            Ok(())
        }
        "audit" => {
            if !has_metrics {
                return Err("sidecar: audit requires a recorded metrics stage".to_string());
            }
            Ok(())
        }
        "verdict" => {
            if !has_metrics {
                return Err("sidecar: verdict requires a recorded metrics stage".to_string());
            }
            let verdict = record.get("verdict").and_then(Value::as_str).unwrap_or("");
            if !matches!(verdict, "adopt-topic" | "iterate" | "abandon") {
                return Err(format!(
                    "sidecar: verdict must be adopt-topic | iterate | abandon, got {:?}",
                    verdict
                ));
            }
            if verdict == "adopt-topic" {
                let last_audit_passed = existing
                    .iter()
                    .rev()
                    .find(|v| stage_of(v) == "audit")
                    .and_then(|v| v.get("passed_bar").and_then(Value::as_bool))
                    .unwrap_or(false);
                if !last_audit_passed {
                    return Err(
                        "sidecar: adopt-topic requires the last recorded audit to have PASSED \
                         the pre-registered bar (fail-closed)"
                            .to_string(),
                    );
                }
            }
            Ok(())
        }
        other => Err(format!("sidecar: unknown stage {:?}", other)),
    }
}

// --------------------------------------------------------------------------------------
// Seed registry (data authored by sq-tzars.3)
// --------------------------------------------------------------------------------------

/// One seed from `ingest/literature-seeds.toml` (the registry `sq-tzars.3` authored;
/// validated by `ingest/validate_seeds.py`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedSpec {
    /// The unique seed id.
    pub id: String,
    /// The registered topic.
    pub topic: String,
    /// The sparq attribute the seed serves.
    pub attribute: String,
    /// The literature-API search query.
    pub query: String,
    /// The seed's own record budget (the run takes `min(seed, caps)`).
    pub max_records: usize,
}

/// Parse the `[[seeds]]` registry (a minimal, dep-free reader for the same constrained
/// subset `validate_seeds.py`'s fallback parser reads: one `key = value` per line, string
/// or integer values, `#` comments).
pub fn parse_seed_registry(toml_str: &str) -> Result<Vec<SeedSpec>, String> {
    let mut seeds: Vec<BTreeMap<String, String>> = Vec::new();
    let mut current: Option<BTreeMap<String, String>> = None;
    for line in toml_str.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[seeds]]" {
            if let Some(c) = current.take() {
                seeds.push(c);
            }
            current = Some(BTreeMap::new());
            continue;
        }
        if let (Some(c), Some(eq)) = (current.as_mut(), line.find('=')) {
            let key = line[..eq].trim().to_string();
            let mut val = line[eq + 1..].trim().to_string();
            if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
            {
                val = val[1..val.len() - 1].to_string();
            }
            c.insert(key, val);
        }
    }
    if let Some(c) = current.take() {
        seeds.push(c);
    }
    let mut out = Vec::new();
    for (i, s) in seeds.iter().enumerate() {
        let get = |k: &str| {
            s.get(k)
                .cloned()
                .ok_or_else(|| format!("seed registry: entry {} missing `{}`", i, k))
        };
        let max_records: usize = get("max_records")?
            .parse()
            .map_err(|e| format!("seed registry: entry {} max_records: {}", i, e))?;
        out.push(SeedSpec {
            id: get("id")?,
            topic: get("topic")?,
            attribute: get("attribute")?,
            query: get("query")?,
            max_records,
        });
    }
    Ok(out)
}

// --------------------------------------------------------------------------------------
// The pilot run (type-state: prereg-first)
// --------------------------------------------------------------------------------------

/// The pilot run configuration, recorded verbatim in the `prereg` sidecar record.
#[derive(Debug, Clone)]
pub struct PilotConfig {
    /// The unique run id (also the staging-dir + finding-provenance key).
    pub run_id: String,
    /// The seed driving this run.
    pub seed: SeedSpec,
    /// The literature source id (`openalex` | `core` | `fixture`; the Semantic Scholar
    /// slot is registered but gated — see the binary).
    pub source: String,
    /// The per-run hard caps (fail-stop; see [`CapLedger`]).
    pub caps: HardCaps,
    /// The pre-registered audit bar.
    pub bar: PreregBar,
    /// Whether the bar was maintainer-steered away from the default (recorded so an
    /// override is always visible in the sidecar).
    pub bar_overridden: bool,
    /// Dry-run flag. `true` is the DEFAULT and the only mode this crate ships enabled;
    /// a live (KB-mutating) emit is a separate maintainer-armed action gated by
    /// [`live_emit_allowed`], never part of the run itself.
    pub dry_run: bool,
    /// UTC ISO-8601 creation instant (injectable for deterministic tests).
    pub created_at: String,
}

/// The pluggable SHACL gate the dry run applies to each emitted tier artifact (the
/// binary wires the real `sparq-shacl` validator behind the `validate` feature; pure
/// tests wire a stand-in). `Ok(true)` = the artifact conforms.
pub type ShaclGate<'a> = &'a dyn Fn(&str) -> Result<bool, String>;

/// A pre-registered pilot run: the ONLY handle through which extraction and metrics
/// recording are reachable. Constructing it via [`PilotRun::preregister`] appends the
/// `prereg` record BEFORE returning, so the prereg-before-extraction ordering is enforced
/// by the type system, not by caller discipline.
#[derive(Debug)]
pub struct PilotRun {
    cfg: PilotConfig,
    sidecar: SidecarFile,
}

impl PilotRun {
    /// Pre-register the run: append the `prereg` record (bar + caps + seed + notices) to
    /// the sidecar FIRST, then return the run handle. Fails if the sidecar already has
    /// records (one sidecar per run) or cannot be written.
    pub fn preregister(cfg: PilotConfig, sidecar_path: PathBuf) -> Result<Self, String> {
        let sidecar = SidecarFile::new(sidecar_path);
        let caps = &cfg.caps;
        sidecar.append(json!({
            "stage": "prereg",
            "schema": SIDECAR_SCHEMA,
            "run_id": cfg.run_id,
            "seed": {
                "id": cfg.seed.id,
                "topic": cfg.seed.topic,
                "attribute": cfg.seed.attribute,
                "query": cfg.seed.query,
                "seed_max_records": cfg.seed.max_records,
            },
            "source": cfg.source,
            "dry_run": cfg.dry_run,
            "bar": {
                "min_precision": cfg.bar.min_precision,
                "min_sample": cfg.bar.min_sample,
                "overridden": cfg.bar_overridden,
                "note": "pre-registered threshold choice, NOT a measured claim",
            },
            "caps": {
                "max_records": caps.max_records,
                "max_api_requests": caps.max_api_requests,
                "max_api_requests_per_day": caps.max_api_requests_per_day,
                "max_subagent_invocations": caps.max_subagent_invocations,
                "max_prompt_bytes_per_invocation": caps.max_prompt_bytes_per_invocation,
                "max_prompt_bytes_total": caps.max_prompt_bytes_total,
            },
            "created_at": cfg.created_at,
            "confidence_notice": UNCALIBRATED_NOTICE,
            "timing_notice": "work-box run; timings are non-canonical and not recorded as claims",
        }))?;
        Ok(Self { cfg, sidecar })
    }

    /// The run configuration.
    pub fn config(&self) -> &PilotConfig {
        &self.cfg
    }

    /// The sidecar file.
    pub fn sidecar(&self) -> &SidecarFile {
        &self.sidecar
    }

    /// Batched extraction under the cap ledger. Reachable ONLY through a pre-registered
    /// run (the prereg-before-extraction invariant). FAIL-STOP: the whole batch budget is
    /// pre-checked BEFORE the first invocation — if the batches needed exceed the
    /// remaining invocation budget, NOTHING is invoked and the run aborts (never a silent
    /// partial extraction).
    pub fn extract_batched<E: Extractor>(
        &self,
        extractor: &E,
        stubs: &[SourceStub],
        batch_size: usize,
        ledger: &mut CapLedger,
    ) -> Result<(Vec<CandidateFinding>, usize), String> {
        if batch_size == 0 {
            return Err("extract_batched: batch_size must be positive".to_string());
        }
        let batches_needed = stubs.chunks(batch_size).count();
        if batches_needed > ledger.remaining_invocations() {
            return Err(format!(
                "cap fail-stop: {} sub-agent batches needed but only {} invocations remain — \
                 aborting BEFORE any invocation (reduce max_records or raise the cap)",
                batches_needed,
                ledger.remaining_invocations()
            ));
        }
        let mut all = Vec::new();
        let mut invocations = 0usize;
        for chunk in stubs.chunks(batch_size) {
            let prompt = build_batch_prompt(chunk);
            ledger.try_charge_invocation(prompt.len())?;
            invocations += 1;
            all.extend(extractor.extract(chunk)?);
        }
        Ok((all, invocations))
    }

    /// Execute the DRY run over an already-fetched batch: extract (batched, capped) →
    /// ground → tier → optional SHACL gate → staging artifacts → metrics record. Writes
    /// ONLY to `staging_dir` (when given) and the sidecar — never to the KB. On ANY
    /// failure after prereg, an error `metrics` record is appended (the run always leaves
    /// an honest trace) and the error is returned.
    pub fn execute_dry<E: Extractor>(
        &self,
        inputs: &DryRunInputs<'_>,
        extractor: &E,
        ledger: &mut CapLedger,
        shacl_gate: Option<ShaclGate<'_>>,
    ) -> Result<RunMetrics, String> {
        match self.execute_dry_inner(inputs, extractor, ledger, shacl_gate) {
            Ok(m) => {
                self.record_metrics(&m)?;
                Ok(m)
            }
            Err(e) => {
                let mut m = RunMetrics::empty(&inputs.fetch);
                m.error = Some(e.clone());
                m.subagent_invocations = ledger.invocations_used();
                m.prompt_bytes_total = ledger.prompt_bytes_used();
                m.set_boundary_rejects(&extractor.boundary_rejects());
                // Best-effort trace: if even the metrics append fails, surface the
                // original error (the append error is secondary).
                let _ = self.record_metrics(&m);
                Err(e)
            }
        }
    }

    fn execute_dry_inner<E: Extractor>(
        &self,
        inputs: &DryRunInputs<'_>,
        extractor: &E,
        ledger: &mut CapLedger,
        shacl_gate: Option<ShaclGate<'_>>,
    ) -> Result<RunMetrics, String> {
        let (stubs, skipped) = parse_openalex_batch(inputs.batch_json)?;
        ledger.try_charge_records(stubs.len())?;
        let (candidates, invocations) =
            self.extract_batched(extractor, &stubs, inputs.batch_size, ledger)?;

        let completeness = if inputs.fetch.complete && !inputs.fetch.records_capped {
            BatchCompleteness::Complete
        } else {
            BatchCompleteness::CapTruncated {
                pages_fetched: inputs.fetch.pages_fetched,
                max_pages: inputs.fetch.pages_fetched,
            }
        };
        let pre = PreExtracted(candidates);
        let tiered = pipeline::run_tiered(
            inputs.batch_json,
            &pre,
            inputs.generated_at_time.clone(),
            completeness,
        )?;

        // SHACL gate (when the caller supplies one — the binary does under `validate`).
        let (shacl_checked, shacl_conforms) = match shacl_gate {
            None => (false, None),
            Some(gate) => {
                let machine_ok = gate(&tiered.machine_tier)?;
                let restricted_ok = gate(&tiered.license_restricted_tier)?;
                (true, Some(machine_ok && restricted_ok))
            }
        };

        let machine_findings = list_findings(&tiered.machine_tier);
        let restricted_findings = list_findings(&tiered.license_restricted_tier);
        let mut all_findings: Vec<FindingRef> = Vec::new();
        all_findings.extend(machine_findings.iter().cloned());
        all_findings.extend(restricted_findings.iter().cloned());

        let sample_seed = fnv1a64(self.cfg.run_id.as_bytes());
        let sample = select_uniform_sample(&all_findings, self.cfg.bar.min_sample, sample_seed);

        // Staging artifacts (LOCAL only — the dry run's whole write surface).
        if let Some(dir) = inputs.staging_dir {
            std::fs::create_dir_all(dir).map_err(|e| format!("staging dir: {}", e))?;
            let w = |name: &str, content: &str| {
                std::fs::write(dir.join(name), content)
                    .map_err(|e| format!("staging write {}: {}", name, e))
            };
            w("machine-tier.ttl", &tiered.machine_tier)?;
            w("license-restricted-tier.ttl", &tiered.license_restricted_tier)?;
            w(
                "restricted-public-projection.ttl",
                &tiered.restricted_public_projection,
            )?;
            w("combined-batch.json", inputs.batch_json)?;
            // The RUN-level DQV quality measurements (sq-2489d.5, design §4.5): the
            // batch's grounding + source-yield rates as `dqv:QualityMeasurement` triples,
            // so "how good was this batch?" is answerable by the `batch-quality` canned
            // query over the artifact rather than only by reading the metrics record.
            // Its own artifact, mirroring the tier separation: run telemetry is not graph
            // content.
            w(
                "batch-quality.ttl",
                &tiered.sidecar.quality_measurements_turtle(
                    &super::pipeline::batch_iri(&self.cfg.run_id),
                    &tiered.generated_at_time,
                )?,
            )?;
            w(
                "run-provenance.ttl",
                &emit_run_provenance(
                    &self.cfg.run_id,
                    &self.cfg.seed.id,
                    &self.cfg.source,
                    inputs.source_endpoint,
                    &tiered.generated_at_time,
                    self.cfg.dry_run,
                ),
            )?;
            // The audit sample WITH justifications (abstract-derived text) stays in the
            // LOCAL staging area only — never in the committed sidecar (fail-closed
            // licensing). The auditor reads this file.
            let audit_items: Vec<Value> = sample
                .iter()
                .map(|f| {
                    json!({
                        "finding": f.finding_iri,
                        "source": f.source_iri,
                        "justification": f.justification,
                    })
                })
                .collect();
            w(
                "audit-sample.json",
                &serde_json::to_string_pretty(&json!({
                    "run_id": self.cfg.run_id,
                    "bar": { "min_precision": self.cfg.bar.min_precision,
                             "min_sample": self.cfg.bar.min_sample },
                    "sample_seed": format!("{:016x}", sample_seed),
                    "items": audit_items,
                }))
                .map_err(|e| e.to_string())?,
            )?;
        }

        let sc = &tiered.sidecar;
        let mut m = RunMetrics {
            fetched_records: inputs.fetch.fetched_records,
            skipped_records: skipped,
            api_requests: inputs.fetch.api_requests,
            pages_fetched: inputs.fetch.pages_fetched,
            batch_complete: completeness.is_complete(),
            subagent_invocations: invocations,
            prompt_bytes_total: ledger.prompt_bytes_used(),
            boundary_rejects: 0,
            boundary_reject_reasons: BTreeMap::new(),
            boundary_rejects_detail: Vec::new(),
            candidates_total: sc.candidates_total,
            grounded: sc.grounded,
            grounding_rate: sc.grounding_rate(),
            quarantined: sc.quarantined.len(),
            quarantine_reasons: quarantine_reason_counts(sc),
            quarantined_detail: sc
                .quarantined
                .iter()
                .map(|q| (q.source_doi.clone(), categorize_reason(&q.reason).to_string()))
                .collect(),
            shacl_checked,
            shacl_conforms,
            machine_tier_findings: machine_findings.len(),
            license_restricted_findings: restricted_findings.len(),
            sources_explored: sc.sources_explored,
            sources_dead_end: sc.sources_dead_end,
            audit_sample: sample
                .iter()
                .map(|f| (f.finding_iri.clone(), f.source_iri.clone()))
                .collect(),
            sample_seed,
            staging_dir: inputs
                .staging_dir
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            error: None,
        };
        // The extractor's own defensive-boundary drops (sq-tx8v9): counted into the
        // metrics record so the fetched→extracted honesty gap is never invisible.
        m.set_boundary_rejects(&extractor.boundary_rejects());
        Ok(m)
    }

    /// Append the `metrics` record (verbatim, append-only).
    pub fn record_metrics(&self, m: &RunMetrics) -> Result<(), String> {
        self.sidecar.append(m.to_json())
    }

    /// Append an `audit` record. The pass/fail is computed against the bar READ BACK from
    /// the sidecar's own `prereg` record (the pre-registered bar is authoritative — a
    /// caller cannot substitute a softer bar post-hoc). `audited_precision` is recorded
    /// VERBATIM. Returns whether the bar passed.
    pub fn record_audit(
        &self,
        audited_n: usize,
        audited_precision: f64,
        auditor: &str,
    ) -> Result<bool, String> {
        record_audit_at(&self.sidecar, audited_n, audited_precision, auditor)
    }

    /// Append the terminal `verdict` record (`adopt-topic | iterate | abandon`).
    /// `changes_before_next_iteration` notes the extract/ground change the next iteration
    /// will carry (the loop's auditability requirement).
    pub fn record_verdict(
        &self,
        verdict: &str,
        notes: &str,
        changes_before_next_iteration: &str,
    ) -> Result<(), String> {
        record_verdict_at(&self.sidecar, verdict, notes, changes_before_next_iteration)
    }
}

/// Append an `audit` record to an existing sidecar (the standalone form the binary's
/// `record-audit` subcommand uses — audits typically happen AFTER the run process exits).
/// Pass/fail is computed against the sidecar's own pre-registered bar.
pub fn record_audit_at(
    sidecar: &SidecarFile,
    audited_n: usize,
    audited_precision: f64,
    auditor: &str,
) -> Result<bool, String> {
    let chain = sidecar.chain()?;
    let prereg = chain
        .iter()
        .find(|v| v.get("stage").and_then(Value::as_str) == Some("prereg"))
        .ok_or_else(|| "audit: sidecar has no prereg record".to_string())?;
    let bar = prereg
        .get("bar")
        .ok_or_else(|| "audit: prereg record has no bar".to_string())?;
    let min_precision = bar
        .get("min_precision")
        .and_then(Value::as_f64)
        .ok_or_else(|| "audit: bar has no min_precision".to_string())?;
    let min_sample = bar
        .get("min_sample")
        .and_then(Value::as_u64)
        .ok_or_else(|| "audit: bar has no min_sample".to_string())? as usize;
    let passed = audited_n >= min_sample && audited_precision >= min_precision;
    sidecar.append(json!({
        "stage": "audit",
        "audited_n": audited_n,
        "audited_precision": audited_precision,
        "auditor": auditor,
        "bar_min_precision": min_precision,
        "bar_min_sample": min_sample,
        "passed_bar": passed,
    }))?;
    Ok(passed)
}

/// Append the terminal `verdict` record to an existing sidecar (standalone form).
pub fn record_verdict_at(
    sidecar: &SidecarFile,
    verdict: &str,
    notes: &str,
    changes_before_next_iteration: &str,
) -> Result<(), String> {
    sidecar.append(json!({
        "stage": "verdict",
        "verdict": verdict,
        "notes": notes,
        "changes_before_next_iteration": changes_before_next_iteration,
    }))
}

/// An extractor that replays a pre-extracted candidate set (the pilot extracts in capped
/// batches FIRST, then feeds the pipeline through this so `run_tiered` needs no second
/// model call).
struct PreExtracted(Vec<CandidateFinding>);

impl Extractor for PreExtracted {
    fn extract(&self, _stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String> {
        Ok(self.0.clone())
    }
}

// --------------------------------------------------------------------------------------
// Fetch stats + run metrics
// --------------------------------------------------------------------------------------

/// What the (binary-side) fetch stage reports into the metrics record. The pure pilot
/// never fetches; the binary fills this from the live connector run (or zeros for a
/// fixture batch).
#[derive(Debug, Clone, Copy, Default)]
pub struct FetchStats {
    /// Raw records fetched across all pages (before parse-time skips).
    pub fetched_records: usize,
    /// Literature-API HTTP requests made (incl. retries).
    pub api_requests: usize,
    /// Pages fetched.
    pub pages_fetched: usize,
    /// Whether the source reported the result set as exhausted.
    pub complete: bool,
    /// Whether the record hard-cap truncated the batch (⇒ incomplete artifact markers).
    pub records_capped: bool,
}

/// One run's honest iteration metrics — recorded VERBATIM in the sidecar. Deliberately
/// carries NO abstract-derived text (only counts, rates, DOIs, IRIs, reasons), so the
/// committed sidecar is safe under the fail-closed licensing posture.
#[derive(Debug, Clone, PartialEq)]
pub struct RunMetrics {
    /// Raw records fetched.
    pub fetched_records: usize,
    /// Records skipped at parse (no DOI / no title) — surfaced, never lost.
    pub skipped_records: usize,
    /// API requests made.
    pub api_requests: usize,
    /// Pages fetched.
    pub pages_fetched: usize,
    /// Whether the batch was complete (false ⇒ artifacts carry INCOMPLETE markers).
    pub batch_complete: bool,
    /// Sub-agent invocations made.
    pub subagent_invocations: usize,
    /// Total prompt bytes sent (the cost-ceiling proxy).
    pub prompt_bytes_total: usize,
    /// Candidates the extractor REJECTED at its own defensive boundary (likely
    /// hallucinations dropped BEFORE the pipeline — `sq-tx8v9`: counted, never silent;
    /// `candidates_total` below counts only the survivors, so `grounding_rate` alone
    /// cannot show these drops).
    pub boundary_rejects: usize,
    /// Boundary-reject reason-category → count.
    pub boundary_reject_reasons: BTreeMap<String, usize>,
    /// Per-boundary-reject `(source DOI, reason category)` — the "rejected + why" trace
    /// at the extraction boundary (no justification text).
    pub boundary_rejects_detail: Vec<(String, String)>,
    /// Candidate findings the extractor proposed.
    pub candidates_total: usize,
    /// Candidates that passed the deterministic grounding-resolver.
    pub grounded: usize,
    /// grounded / candidates_total (1.0 when no candidates — vacuous).
    pub grounding_rate: f64,
    /// Candidates quarantined (never dropped).
    pub quarantined: usize,
    /// Quarantine reason-category → count.
    pub quarantine_reasons: BTreeMap<String, usize>,
    /// Per-quarantined-candidate `(source DOI, reason category)` — the "rejected + why"
    /// trace (no justification text).
    pub quarantined_detail: Vec<(String, String)>,
    /// Whether a SHACL gate ran (requires the `validate` feature in the caller).
    pub shacl_checked: bool,
    /// SHACL conformance of the emitted tier artifacts (None when not checked).
    pub shacl_conforms: Option<bool>,
    /// Findings emitted into the machine tier (redistributable-licensed sources).
    pub machine_tier_findings: usize,
    /// Findings emitted into the license-restricted tier (fail-closed licensing).
    pub license_restricted_findings: usize,
    /// Sources with ≥ 1 grounded finding.
    pub sources_explored: usize,
    /// Sources with 0 grounded findings.
    pub sources_dead_end: usize,
    /// The uniform audit sample: `(finding IRI, source IRI)` pairs (no text).
    pub audit_sample: Vec<(String, String)>,
    /// The deterministic sample seed (FNV-1a of the run id).
    pub sample_seed: u64,
    /// The LOCAL staging directory the artifacts went to (dry-run write surface).
    pub staging_dir: String,
    /// A fail-stop / pipeline error, when the run aborted (partial counts above).
    pub error: Option<String>,
}

impl RunMetrics {
    fn empty(fetch: &FetchStats) -> Self {
        Self {
            fetched_records: fetch.fetched_records,
            skipped_records: 0,
            api_requests: fetch.api_requests,
            pages_fetched: fetch.pages_fetched,
            batch_complete: fetch.complete && !fetch.records_capped,
            subagent_invocations: 0,
            prompt_bytes_total: 0,
            boundary_rejects: 0,
            boundary_reject_reasons: BTreeMap::new(),
            boundary_rejects_detail: Vec::new(),
            candidates_total: 0,
            grounded: 0,
            grounding_rate: 0.0,
            quarantined: 0,
            quarantine_reasons: BTreeMap::new(),
            quarantined_detail: Vec::new(),
            shacl_checked: false,
            shacl_conforms: None,
            machine_tier_findings: 0,
            license_restricted_findings: 0,
            sources_explored: 0,
            sources_dead_end: 0,
            audit_sample: Vec::new(),
            sample_seed: 0,
            staging_dir: String::new(),
            error: None,
        }
    }

    /// Record the extractor's defensive-boundary drops into the metrics (`sq-tx8v9`:
    /// boundary rejects are COUNTED, never silently dropped). Only the source DOI +
    /// stable reason category are carried — never any abstract-derived text, so the
    /// committed sidecar stays licence-safe.
    pub fn set_boundary_rejects(&mut self, rejects: &[BoundaryReject]) {
        self.boundary_rejects = rejects.len();
        let mut reasons = BTreeMap::new();
        for r in rejects {
            *reasons.entry(r.reason.clone()).or_insert(0) += 1;
        }
        self.boundary_reject_reasons = reasons;
        self.boundary_rejects_detail = rejects
            .iter()
            .map(|r| (r.source_doi.clone(), r.reason.clone()))
            .collect();
    }

    /// The sidecar `metrics` record (append-only JSON; carries the structural
    /// [`UNCALIBRATED_NOTICE`]).
    pub fn to_json(&self) -> Value {
        json!({
            "stage": "metrics",
            "fetched_records": self.fetched_records,
            "skipped_records": self.skipped_records,
            "api_requests": self.api_requests,
            "pages_fetched": self.pages_fetched,
            "batch_complete": self.batch_complete,
            "subagent_invocations": self.subagent_invocations,
            "prompt_bytes_total": self.prompt_bytes_total,
            "boundary_rejects": self.boundary_rejects,
            "boundary_reject_reasons": self.boundary_reject_reasons,
            "boundary_rejects_detail": self.boundary_rejects_detail.iter()
                .map(|(doi, reason)| json!({ "source_doi": doi, "reason": reason }))
                .collect::<Vec<_>>(),
            "candidates_total": self.candidates_total,
            "grounded": self.grounded,
            "grounding_rate": self.grounding_rate,
            "quarantined": self.quarantined,
            "quarantine_reasons": self.quarantine_reasons,
            "quarantined_detail": self.quarantined_detail.iter()
                .map(|(doi, reason)| json!({ "source_doi": doi, "reason": reason }))
                .collect::<Vec<_>>(),
            "shacl_checked": self.shacl_checked,
            "shacl_conforms": self.shacl_conforms,
            "machine_tier_findings": self.machine_tier_findings,
            "license_restricted_findings": self.license_restricted_findings,
            "sources_explored": self.sources_explored,
            "sources_dead_end": self.sources_dead_end,
            "audit_sample": self.audit_sample.iter()
                .map(|(f, s)| json!({ "finding": f, "source": s }))
                .collect::<Vec<_>>(),
            "sample_seed": format!("{:016x}", self.sample_seed),
            "staging_dir": self.staging_dir,
            "error": self.error,
            "confidence_notice": UNCALIBRATED_NOTICE,
        })
    }
}

/// Collapse a pipeline quarantine reason string to a stable category key.
pub fn categorize_reason(reason: &str) -> &'static str {
    if reason.contains("not a span") {
        "justification-not-anchored"
    } else if reason.contains("does not resolve") || reason.contains("dangling") {
        "dangling-citation"
    } else if reason.contains("not in batch") {
        "unknown-source"
    } else {
        "other"
    }
}

/// Reason-category → count over a pipeline sidecar's quarantine list.
pub fn quarantine_reason_counts(sc: &pipeline::Sidecar) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for q in &sc.quarantined {
        *m.entry(categorize_reason(&q.reason).to_string()).or_insert(0) += 1;
    }
    m
}

// --------------------------------------------------------------------------------------
// Findings enumeration + uniform sampling
// --------------------------------------------------------------------------------------

/// A grounded finding reference scraped from an emitted tier artifact: enough for the
/// audit sample (IRI + source + justification), nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingRef {
    /// The finding IRI (stable across tiers by construction).
    pub finding_iri: String,
    /// The `prov:wasDerivedFrom` source IRI (the DOI resolver form).
    pub source_iri: String,
    /// The (escaped) justification literal — LOCAL staging use only.
    pub justification: String,
}

/// Enumerate the `pkg:Finding` nodes in an emitted tier artifact (the emitter's
/// single-writer layout: `<iri> a pkg:Finding ;` then `sigimpl:justification` +
/// `prov:wasDerivedFrom` property lines). The artifacts are the authoritative emission,
/// so counting here cannot drift from what was actually emitted.
pub fn list_findings(ttl: &str) -> Vec<FindingRef> {
    let mut out = Vec::new();
    let mut current: Option<(String, Option<String>, Option<String>)> = None;
    for line in ttl.lines() {
        let t = line.trim();
        if t.starts_with('<') && t.contains("> a pkg:Finding") {
            if let Some(end) = t.find('>') {
                current = Some((t[1..end].to_string(), None, None));
            }
            continue;
        }
        if let Some((iri, just, src)) = current.as_mut() {
            if let Some(rest) = t.strip_prefix("sigimpl:justification \"") {
                if let Some(end) = rest.rfind('"') {
                    *just = Some(rest[..end].to_string());
                }
            } else if let Some(rest) = t.strip_prefix("prov:wasDerivedFrom <") {
                if let Some(end) = rest.find('>') {
                    *src = Some(rest[..end].to_string());
                }
            }
            if let (Some(j), Some(s)) = (just.clone(), src.clone()) {
                out.push(FindingRef {
                    finding_iri: iri.clone(),
                    source_iri: s,
                    justification: j,
                });
                current = None;
            }
        }
    }
    out
}

/// Select a deterministic uniform sample of up to `target` findings (partial Fisher-Yates
/// over a seeded LCG). The seed is recorded in the sidecar so the sample is reproducible;
/// when fewer than `target` findings exist, ALL are returned (and the bar then cannot
/// pass — the audit abstains rather than shrinking its own sample requirement).
pub fn select_uniform_sample(findings: &[FindingRef], target: usize, seed: u64) -> Vec<FindingRef> {
    let mut idx: Vec<usize> = (0..findings.len()).collect();
    // NOTE: no `seed | 1` here — a mixed-congruential LCG accepts any state (including
    // 0), and forcing the low bit would collapse adjacent seeds (42 and 43) onto one
    // stream (caught by `uniform_sample_is_deterministic_and_bounded`). [FABLE-5]
    let mut state = seed;
    let n = target.min(idx.len());
    for i in 0..n {
        // LCG step (Knuth MMIX constants) — deterministic, dep-free.
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let j = i + (state >> 33) as usize % (idx.len() - i);
        idx.swap(i, j);
    }
    idx[..n].iter().map(|&i| findings[i].clone()).collect()
}

// --------------------------------------------------------------------------------------
// PROV-O run provenance
// --------------------------------------------------------------------------------------

/// Emit the run's own PROV-O record (the retrieval `prov:Activity` — the estate pattern):
/// the pilot run as an Activity that `prov:used` the source endpoint (never any key) and
/// was associated with the machine agent, stamped `prov:generatedAtTime`.
pub fn emit_run_provenance(
    run_id: &str,
    seed_id: &str,
    source: &str,
    source_endpoint: &str,
    generated_at_time: &str,
    dry_run: bool,
) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut t = String::new();
    t.push_str(
        "@prefix pkg:  <https://sparq.dev/ns/pkg#> .\n\
         @prefix prov: <http://www.w3.org/ns/prov#> .\n\
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
         @prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .\n\n",
    );
    let _ = writeln!(
        t,
        "<{agent}/pilot-run/{run}> a prov:Activity ;\n  \
         rdfs:label \"literature pilot run {run_label} (dry_run={dry})\"@en ;\n  \
         prov:used <{endpoint}> ;\n  \
         prov:wasAssociatedWith <{agent}> ;\n  \
         prov:generatedAtTime \"{ts}\"^^xsd:dateTime ;\n  \
         rdfs:comment \"seed={seed}; source={source}; {notice}\"@en .",
        agent = MACHINE_AGENT_IRI,
        run = run_id,
        run_label = esc(run_id),
        dry = dry_run,
        endpoint = source_endpoint,
        ts = generated_at_time,
        seed = esc(seed_id),
        source = esc(source),
        notice = esc(UNCALIBRATED_NOTICE),
    );
    t
}

// --------------------------------------------------------------------------------------
// The dry-run inputs + live-emit gate
// --------------------------------------------------------------------------------------

/// The inputs to [`PilotRun::execute_dry`]: an already-fetched (or fixture) batch plus
/// the fetch stats and staging destination.
#[derive(Debug, Clone)]
pub struct DryRunInputs<'a> {
    /// The combined OpenAlex-shaped batch JSON (`{"results": [...]}`).
    pub batch_json: &'a str,
    /// The fetch-stage stats (zeros for a fixture batch).
    pub fetch: FetchStats,
    /// Stubs per sub-agent batch.
    pub batch_size: usize,
    /// The LOCAL staging directory (the dry run's only artifact surface). `None` skips
    /// artifact writes (metrics only — used by tests).
    pub staging_dir: Option<&'a Path>,
    /// Injectable `prov:generatedAtTime` (tests pass a fixed instant).
    pub generated_at_time: Option<String>,
    /// The source endpoint IRI recorded in the run provenance (never carries a key).
    pub source_endpoint: &'a str,
}

/// The MAINTAINER-ARM live-emit gate — the load-bearing invariant of `sq-tzars.9`: **the
/// KB is never mutated before the pre-registered bar passes.** Returns `Ok(())` only if
/// the sidecar chain verifies AND has a prereg record AND an error-free, SHACL-checked,
/// SHACL-conformant metrics record AND a recorded audit whose sample size + precision
/// clear the PRE-REGISTERED bar AND no non-`adopt-topic` verdict. Anything else is a
/// fail-closed `Err`.
pub fn live_emit_allowed(sidecar_raw: &str) -> Result<(), String> {
    let chain = verify_chain(sidecar_raw)?;
    fn stage_of(v: &Value) -> &str {
        v.get("stage").and_then(Value::as_str).unwrap_or("")
    }
    let prereg = chain
        .iter()
        .find(|v| stage_of(v) == "prereg")
        .ok_or_else(|| "live-emit gate: no prereg record (bar never pre-registered)".to_string())?;
    let bar = prereg
        .get("bar")
        .ok_or_else(|| "live-emit gate: prereg has no bar".to_string())?;
    let min_precision = bar
        .get("min_precision")
        .and_then(Value::as_f64)
        .ok_or_else(|| "live-emit gate: bar has no min_precision".to_string())?;
    let min_sample = bar
        .get("min_sample")
        .and_then(Value::as_u64)
        .ok_or_else(|| "live-emit gate: bar has no min_sample".to_string())?
        as usize;

    let metrics = chain
        .iter()
        .find(|v| stage_of(v) == "metrics")
        .ok_or_else(|| "live-emit gate: no metrics record".to_string())?;
    if let Some(err) = metrics.get("error").and_then(Value::as_str) {
        return Err(format!("live-emit gate: the run recorded an error: {}", err));
    }
    if metrics.get("shacl_checked").and_then(Value::as_bool) != Some(true)
        || metrics.get("shacl_conforms").and_then(Value::as_bool) != Some(true)
    {
        return Err(
            "live-emit gate: metrics record lacks a passing SHACL check (run with the \
             `validate` feature so the gate is real)"
                .to_string(),
        );
    }

    let audit = chain
        .iter()
        .rev()
        .find(|v| stage_of(v) == "audit")
        .ok_or_else(|| {
            "live-emit gate: NO recorded audit — the dry-run default holds until a \
             human/agent audit is recorded and passes the pre-registered bar"
                .to_string()
        })?;
    let audited_n = audit.get("audited_n").and_then(Value::as_u64).unwrap_or(0) as usize;
    let audited_precision = audit
        .get("audited_precision")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if audited_n < min_sample {
        return Err(format!(
            "live-emit gate: audited sample of {} is below the pre-registered minimum of {}",
            audited_n, min_sample
        ));
    }
    if audited_precision < min_precision {
        return Err(format!(
            "live-emit gate: audited precision {} is below the pre-registered bar of {}",
            audited_precision, min_precision
        ));
    }
    if let Some(v) = chain.iter().find(|v| stage_of(v) == "verdict") {
        let verdict = v.get("verdict").and_then(Value::as_str).unwrap_or("");
        if verdict != "adopt-topic" {
            return Err(format!(
                "live-emit gate: the recorded verdict is {:?} — only adopt-topic permits a \
                 live emit",
                verdict
            ));
        }
    }
    Ok(())
}

/// Perform the (maintainer-armed) LIVE emit: write the tier artifacts to the KB-facing
/// `out_dir` — but ONLY if [`live_emit_allowed`] passes on the sidecar. Fail-closed:
/// nothing is written on refusal. This crate ships with the calling flag OFF; the first
/// real invocation is the maintainer's.
pub fn live_emit(sidecar_raw: &str, tiered: &TieredOutput, out_dir: &Path) -> Result<(), String> {
    live_emit_allowed(sidecar_raw)?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("live-emit dir: {}", e))?;
    let w = |name: &str, content: &str| {
        std::fs::write(out_dir.join(name), content)
            .map_err(|e| format!("live-emit write {}: {}", name, e))
    };
    w("machine-tier.ttl", &tiered.machine_tier)?;
    w("license-restricted-tier.ttl", &tiered.license_restricted_tier)?;
    w(
        "restricted-public-projection.ttl",
        &tiered.restricted_public_projection,
    )?;
    Ok(())
}

// --------------------------------------------------------------------------------------
// Live-source batch normalisation (pure; the binary fetches, this shapes)
// --------------------------------------------------------------------------------------

/// Reconstruct a plain abstract from an OpenAlex `abstract_inverted_index` (word →
/// positions). Returns `None` when the value is not the inverted-index shape.
pub fn reconstruct_inverted_abstract(inv: &Value) -> Option<String> {
    let obj = inv.as_object()?;
    let mut words: Vec<(u64, &str)> = Vec::new();
    for (w, positions) in obj {
        for p in positions.as_array()? {
            words.push((p.as_u64()?, w.as_str()));
        }
    }
    words.sort();
    Some(
        words
            .iter()
            .map(|(_, w)| *w)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Normalise one page of a LIVE OpenAlex `/works` response into the connector's
/// OpenAlex-batch record shape (`doi` / `title` / plain `abstract` / `publication_year` /
/// `license`): reconstructs `abstract_inverted_index`, falls back `title` →
/// `display_name`, and reads the licence off `primary_location` → `best_oa_location` →
/// top-level (the same precedence the connector uses; absent = fail-closed `None`
/// downstream). Returns the normalised record list.
pub fn normalise_openalex_records(page_json: &str) -> Result<Vec<Value>, String> {
    let root: Value =
        serde_json::from_str(page_json).map_err(|e| format!("OpenAlex JSON: {}", e))?;
    let results = root
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "OpenAlex JSON: missing `results` array".to_string())?;
    let loc_license = |rec: &Value, key: &str| {
        rec.get(key)
            .and_then(|l| l.get("license"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let mut out = Vec::new();
    for rec in results {
        let title = rec
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| rec.get("display_name").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let abstract_text = rec
            .get("abstract")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                rec.get("abstract_inverted_index")
                    .and_then(reconstruct_inverted_abstract)
            })
            .unwrap_or_default();
        let license = loc_license(rec, "primary_location")
            .or_else(|| loc_license(rec, "best_oa_location"))
            .or_else(|| {
                rec.get("license")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        out.push(json!({
            "doi": rec.get("doi").cloned().unwrap_or(Value::Null),
            "title": title,
            "abstract": abstract_text,
            "publication_year": rec.get("publication_year").cloned().unwrap_or(Value::Null),
            "license": license,
        }));
    }
    Ok(out)
}

/// Shape already-normalised `SourceStub`s (e.g. a CORE connector run) into the same
/// OpenAlex-batch record shape, so every source feeds `run_tiered` identically.
pub fn records_from_stubs(stubs: &[SourceStub]) -> Vec<Value> {
    stubs
        .iter()
        .map(|s| {
            json!({
                "doi": s.doi,
                "title": s.title,
                "abstract": s.abstract_text,
                "publication_year": s.year,
                "license": s.license,
            })
        })
        .collect()
}

/// Combine normalised records into the single batch document the pipeline consumes.
pub fn combine_records(records: &[Value]) -> String {
    json!({ "results": records }).to_string()
}

/// The current UTC date (`YYYY-MM-DD`) for the daily ledger.
pub fn today_utc() -> String {
    current_generated_at_time()
        .chars()
        .take(10)
        .collect::<String>()
}

/// The current UTC instant (ISO-8601 `xsd:dateTime` form) — the run `created_at` stamp
/// (the same clock the pipeline's `prov:generatedAtTime` default uses).
pub fn now_utc_iso() -> String {
    current_generated_at_time()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::literature::extract::RecordedExtractor;
    use crate::literature::FIXTURE_OPENALEX_BATCH;
    use std::cell::Cell;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "sparq-kb-pilot-{}-{}-{}",
            tag,
            std::process::id(),
            fnv1a64(tag.as_bytes())
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn test_cfg(run_id: &str) -> PilotConfig {
        PilotConfig {
            run_id: run_id.to_string(),
            seed: SeedSpec {
                id: "test-seed".to_string(),
                topic: "neurosymbolic AI".to_string(),
                attribute: "genai-self-improvement".to_string(),
                query: "neurosymbolic".to_string(),
                max_records: 10,
            },
            source: "fixture".to_string(),
            caps: HardCaps::default(),
            bar: PreregBar::default(),
            bar_overridden: false,
            dry_run: true,
            created_at: "2026-07-05T12:00:00Z".to_string(),
        }
    }

    fn fixture_inputs<'a>(staging: Option<&'a Path>) -> DryRunInputs<'a> {
        DryRunInputs {
            batch_json: FIXTURE_OPENALEX_BATCH,
            fetch: FetchStats {
                fetched_records: 3,
                api_requests: 0,
                pages_fetched: 0,
                complete: true,
                records_capped: false,
            },
            batch_size: 8,
            staging_dir: staging,
            generated_at_time: Some("2026-07-05T12:00:00Z".to_string()),
            source_endpoint: "https://sparq.dev/ns/pkg/example#fixture",
        }
    }

    // ---- PREREG ordering (acceptance: prereg-written-before-extraction) -------------

    #[test]
    fn prereg_record_is_written_before_any_extraction() {
        let dir = tmpdir("prereg-order");
        let sidecar_path = dir.join("sidecar.jsonl");
        let run = PilotRun::preregister(test_cfg("prereg-order"), sidecar_path.clone()).unwrap();

        // An extractor that PROVES the prereg record is already on disk when invoked.
        struct AssertPrereg<'a> {
            path: &'a Path,
            called: &'a Cell<bool>,
        }
        impl Extractor for AssertPrereg<'_> {
            fn extract(&self, _stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String> {
                let raw = std::fs::read_to_string(self.path).expect("sidecar exists");
                let chain = verify_chain(&raw).expect("chain verifies");
                assert!(
                    chain
                        .iter()
                        .any(|v| v.get("stage").and_then(Value::as_str) == Some("prereg")),
                    "the prereg record MUST be on disk before extraction runs"
                );
                self.called.set(true);
                Ok(Vec::new())
            }
        }
        let called = Cell::new(false);
        let ex = AssertPrereg {
            path: &sidecar_path,
            called: &called,
        };
        let (stubs, _) = parse_openalex_batch(FIXTURE_OPENALEX_BATCH).unwrap();
        let mut ledger = CapLedger::new(HardCaps::default());
        run.extract_batched(&ex, &stubs, 8, &mut ledger).unwrap();
        assert!(called.get(), "the extractor ran (the assertion above fired)");
    }

    #[test]
    fn prereg_carries_bar_caps_and_uncalibrated_notice() {
        let dir = tmpdir("prereg-fields");
        let run = PilotRun::preregister(test_cfg("prereg-fields"), dir.join("s.jsonl")).unwrap();
        let chain = run.sidecar().chain().unwrap();
        assert_eq!(chain.len(), 1);
        let p = &chain[0];
        assert_eq!(p["stage"], "prereg");
        assert_eq!(p["schema"], SIDECAR_SCHEMA);
        assert_eq!(p["bar"]["min_precision"], DEFAULT_BAR_MIN_PRECISION);
        assert_eq!(p["bar"]["min_sample"], DEFAULT_BAR_MIN_SAMPLE);
        assert_eq!(p["bar"]["overridden"], false);
        assert_eq!(p["caps"]["max_records"], HardCaps::default().max_records);
        assert_eq!(p["dry_run"], true);
        assert_eq!(p["confidence_notice"], UNCALIBRATED_NOTICE);
    }

    // ---- HARD CAPS fail-stop (acceptance: cap fail-stop) -----------------------------

    #[test]
    fn cap_ledger_fail_stops_on_every_axis() {
        let caps = HardCaps {
            max_records: 5,
            max_api_requests: 2,
            max_subagent_invocations: 1,
            max_prompt_bytes_per_invocation: 100,
            max_prompt_bytes_total: 150,
            ..HardCaps::default()
        };
        let mut l = CapLedger::new(caps);
        // Records: 5 ok, then +1 refused.
        l.try_charge_records(5).unwrap();
        assert!(l.try_charge_records(1).unwrap_err().contains("cap fail-stop"));
        assert_eq!(l.records_used(), 5, "a refused charge does not count");
        // API requests: 2 ok, third refused.
        l.try_charge_api_request().unwrap();
        l.try_charge_api_request().unwrap();
        assert!(l.try_charge_api_request().unwrap_err().contains("cap fail-stop"));
        // Invocations: 1 ok, second refused.
        l.try_charge_invocation(50).unwrap();
        assert!(l.try_charge_invocation(10).unwrap_err().contains("invocation cap"));
        // Per-invocation prompt ceiling.
        let mut l2 = CapLedger::new(caps);
        assert!(l2.try_charge_invocation(101).unwrap_err().contains("per-invocation"));
        assert_eq!(l2.invocations_used(), 0, "a refused invocation does not count");
    }

    #[test]
    fn extract_batched_fail_stops_before_the_first_invocation() {
        let dir = tmpdir("batch-failstop");
        let run = PilotRun::preregister(test_cfg("batch-failstop"), dir.join("s.jsonl")).unwrap();
        struct MustNotRun;
        impl Extractor for MustNotRun {
            fn extract(&self, _stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String> {
                panic!("cap fail-stop must abort BEFORE any invocation");
            }
        }
        let (stubs, _) = parse_openalex_batch(FIXTURE_OPENALEX_BATCH).unwrap();
        assert!(stubs.len() >= 3);
        let mut ledger = CapLedger::new(HardCaps {
            max_subagent_invocations: 1, // 3 stubs at batch_size=1 need 3 invocations
            ..HardCaps::default()
        });
        let err = run
            .extract_batched(&MustNotRun, &stubs, 1, &mut ledger)
            .unwrap_err();
        assert!(err.contains("aborting BEFORE any invocation"), "got: {}", err);
        assert_eq!(ledger.invocations_used(), 0);
    }

    #[test]
    fn daily_ledger_fail_stops_and_resets_on_a_new_date() {
        let dir = tmpdir("daily");
        let ledger = DailyLedger::new(dir.join("day.json"));
        assert_eq!(ledger.used_today("2026-07-05").unwrap(), 0);
        assert_eq!(ledger.charge("2026-07-05", 8, 10).unwrap(), 8);
        // Over-cap refused, file unchanged.
        assert!(ledger.charge("2026-07-05", 3, 10).unwrap_err().contains("cap fail-stop"));
        assert_eq!(ledger.used_today("2026-07-05").unwrap(), 8);
        // Within cap still fine.
        assert_eq!(ledger.charge("2026-07-05", 2, 10).unwrap(), 10);
        // A new date resets the budget.
        assert_eq!(ledger.used_today("2026-07-06").unwrap(), 0);
        assert_eq!(ledger.charge("2026-07-06", 1, 10).unwrap(), 1);
    }

    // ---- Sidecar append-only chain (acceptance: append-only schema) ------------------

    #[test]
    fn sidecar_chain_detects_retro_editing() {
        let dir = tmpdir("chain-tamper");
        let path = dir.join("s.jsonl");
        let run = PilotRun::preregister(test_cfg("chain-tamper"), path.clone()).unwrap();
        let m = RunMetrics::empty(&FetchStats::default());
        run.record_metrics(&m).unwrap();
        // Verifies clean.
        assert_eq!(run.sidecar().chain().unwrap().len(), 2);
        // Retro-edit the FIRST line (soften the bar) — the chain must break.
        let raw = std::fs::read_to_string(&path).unwrap();
        let tampered = raw.replacen("0.8", "0.1", 1);
        assert_ne!(raw, tampered, "the tamper actually changed the file");
        std::fs::write(&path, &tampered).unwrap();
        let err = run.sidecar().chain().unwrap_err();
        assert!(err.contains("retro-edited"), "got: {}", err);
        // Further appends REFUSE on the broken chain.
        let err = run
            .record_verdict("iterate", "n", "c")
            .unwrap_err();
        assert!(err.contains("retro-edited"), "got: {}", err);
    }

    #[test]
    fn sidecar_stage_order_is_enforced() {
        let dir = tmpdir("stage-order");
        let s = SidecarFile::new(dir.join("s.jsonl"));
        // Metrics before prereg refused.
        let err = s.append(json!({ "stage": "metrics" })).unwrap_err();
        assert!(err.contains("before prereg"), "got: {}", err);
        // Prereg first is fine; a second prereg refused.
        s.append(json!({ "stage": "prereg", "bar": { "min_precision": 0.8, "min_sample": 20 } }))
            .unwrap();
        let err = s.append(json!({ "stage": "prereg" })).unwrap_err();
        assert!(err.contains("FIRST record"), "got: {}", err);
        // Audit before metrics refused.
        let err = s.append(json!({ "stage": "audit" })).unwrap_err();
        assert!(err.contains("requires a recorded metrics"), "got: {}", err);
        s.append(json!({ "stage": "metrics" })).unwrap();
        // Second metrics refused.
        let err = s.append(json!({ "stage": "metrics" })).unwrap_err();
        assert!(err.contains("exactly once"), "got: {}", err);
        // adopt-topic without a passing audit refused (fail-closed).
        let err = s
            .append(json!({ "stage": "verdict", "verdict": "adopt-topic" }))
            .unwrap_err();
        assert!(err.contains("PASSED the pre-registered bar"), "got: {}", err);
        // Unknown verdict refused.
        let err = s
            .append(json!({ "stage": "verdict", "verdict": "ship-it" }))
            .unwrap_err();
        assert!(err.contains("adopt-topic | iterate | abandon"), "got: {}", err);
        // iterate verdict is fine, and is TERMINAL.
        s.append(json!({ "stage": "verdict", "verdict": "iterate" }))
            .unwrap();
        let err = s.append(json!({ "stage": "audit" })).unwrap_err();
        assert!(err.contains("terminal"), "got: {}", err);
    }

    #[test]
    fn audit_pass_is_computed_from_the_preregistered_bar() {
        let dir = tmpdir("audit-bar");
        let run = PilotRun::preregister(test_cfg("audit-bar"), dir.join("s.jsonl")).unwrap();
        run.record_metrics(&RunMetrics::empty(&FetchStats::default()))
            .unwrap();
        // Below the sample minimum: recorded verbatim, does NOT pass.
        assert!(!run.record_audit(5, 1.0, "test").unwrap());
        // Below the precision bar: does NOT pass.
        assert!(!run.record_audit(20, 0.75, "test").unwrap());
        // Clears both: passes.
        assert!(run.record_audit(20, 0.85, "test").unwrap());
        // All three audits recorded verbatim (append-only, no retro-editing).
        let chain = run.sidecar().chain().unwrap();
        let audits: Vec<&Value> = chain
            .iter()
            .filter(|v| v["stage"] == "audit")
            .collect();
        assert_eq!(audits.len(), 3);
        assert_eq!(audits[0]["audited_precision"], 1.0);
        assert_eq!(audits[1]["audited_precision"], 0.75);
        assert_eq!(audits[1]["passed_bar"], false);
        assert_eq!(audits[2]["passed_bar"], true);
        // Now adopt-topic is allowed (last audit passed).
        run.record_verdict("adopt-topic", "bar cleared", "none")
            .unwrap();
    }

    // ---- DRY-RUN gate (acceptance: no emit without a recorded passing audit) --------

    #[test]
    fn live_emit_gate_is_fail_closed() {
        let dir = tmpdir("gate");
        let staging = dir.join("staging");
        let run = PilotRun::preregister(test_cfg("gate"), dir.join("s.jsonl")).unwrap();
        let ex = RecordedExtractor::from_fixture().unwrap();
        let mut ledger = CapLedger::new(HardCaps::default());
        // A SHACL stand-in gate for the pure test (the binary wires the real one).
        let gate = |_ttl: &str| Ok(true);
        run.execute_dry(&fixture_inputs(Some(&staging)), &ex, &mut ledger, Some(&gate))
            .unwrap();

        let raw = || std::fs::read_to_string(run.sidecar().path()).unwrap();
        // NO audit recorded => refused.
        let err = live_emit_allowed(&raw()).unwrap_err();
        assert!(err.contains("NO recorded audit"), "got: {}", err);
        // Failing audit => refused.
        run.record_audit(20, 0.5, "test").unwrap();
        let err = live_emit_allowed(&raw()).unwrap_err();
        assert!(err.contains("below the pre-registered bar"), "got: {}", err);
        // Under-sized sample => refused even at precision 1.0.
        run.record_audit(4, 1.0, "test").unwrap();
        let err = live_emit_allowed(&raw()).unwrap_err();
        assert!(err.contains("below the pre-registered minimum"), "got: {}", err);
        // Passing audit => allowed.
        run.record_audit(20, 0.9, "test").unwrap();
        live_emit_allowed(&raw()).unwrap();
        // A non-adopt verdict blocks again (terminal decision wins).
        run.record_verdict("iterate", "improve grounding", "tighten span check")
            .unwrap();
        let err = live_emit_allowed(&raw()).unwrap_err();
        assert!(err.contains("only adopt-topic"), "got: {}", err);
    }

    #[test]
    fn live_emit_gate_requires_a_real_shacl_check() {
        let dir = tmpdir("gate-shacl");
        let run = PilotRun::preregister(test_cfg("gate-shacl"), dir.join("s.jsonl")).unwrap();
        let ex = RecordedExtractor::from_fixture().unwrap();
        let mut ledger = CapLedger::new(HardCaps::default());
        // NO SHACL gate supplied (shacl_checked=false) — the live emit must refuse.
        run.execute_dry(&fixture_inputs(None), &ex, &mut ledger, None)
            .unwrap();
        run.record_audit(20, 1.0, "test").unwrap();
        let raw = std::fs::read_to_string(run.sidecar().path()).unwrap();
        let err = live_emit_allowed(&raw).unwrap_err();
        assert!(err.contains("SHACL"), "got: {}", err);
    }

    #[test]
    fn live_emit_writes_nothing_when_refused() {
        let dir = tmpdir("live-refuse");
        let out = dir.join("kb-out");
        let run = PilotRun::preregister(test_cfg("live-refuse"), dir.join("s.jsonl")).unwrap();
        let ex = RecordedExtractor::from_fixture().unwrap();
        let mut ledger = CapLedger::new(HardCaps::default());
        let gate = |_ttl: &str| Ok(true);
        run.execute_dry(&fixture_inputs(None), &ex, &mut ledger, Some(&gate))
            .unwrap();
        let (stubs_json, tiered) = {
            let pre = RecordedExtractor::from_fixture().unwrap();
            (
                FIXTURE_OPENALEX_BATCH,
                pipeline::run_tiered(
                    FIXTURE_OPENALEX_BATCH,
                    &pre,
                    Some("2026-07-05T12:00:00Z".to_string()),
                    BatchCompleteness::Complete,
                )
                .unwrap(),
            )
        };
        let _ = stubs_json;
        let raw = std::fs::read_to_string(run.sidecar().path()).unwrap();
        // No audit => live_emit refuses AND writes nothing (fail-closed).
        assert!(live_emit(&raw, &tiered, &out).is_err());
        assert!(!out.exists(), "no KB-facing artifact may exist after a refusal");
    }

    // ---- Fixture-driven end-to-end dry run (acceptance) -------------------------------

    #[test]
    fn end_to_end_fixture_dry_run_produces_prereg_metrics_and_verdict() {
        let dir = tmpdir("e2e");
        let staging = dir.join("staging");
        let run = PilotRun::preregister(test_cfg("e2e"), dir.join("s.jsonl")).unwrap();
        let ex = RecordedExtractor::from_fixture().unwrap();
        let mut ledger = CapLedger::new(HardCaps::default());
        let m = run
            .execute_dry(&fixture_inputs(Some(&staging)), &ex, &mut ledger, None)
            .unwrap();

        // The fixture batch: 3 stubs, 6 candidates, 4 grounded, 2 quarantined.
        assert_eq!(m.candidates_total, 6);
        assert_eq!(m.grounded, 4);
        assert_eq!(m.quarantined, 2);
        assert!((m.grounding_rate - 4.0 / 6.0).abs() < 1e-12);
        assert_eq!(m.subagent_invocations, 1, "3 stubs at batch_size=8 = one batch");
        // Fail-closed licensing: the fixture has no licences, all findings restricted.
        assert_eq!(m.machine_tier_findings, 0);
        assert_eq!(m.license_restricted_findings, 4);
        // The sample abstains-by-construction: only 4 findings < min_sample 20.
        assert_eq!(m.audit_sample.len(), 4);
        // Quarantine reasons categorised, with per-candidate why.
        assert_eq!(m.quarantine_reasons.values().sum::<usize>(), 2);
        assert_eq!(m.quarantined_detail.len(), 2);
        // A replay extractor applies no defensive boundary: zero rejects, recorded
        // honestly (the live-extractor threading is pinned by its own test below).
        assert_eq!(m.boundary_rejects, 0);
        assert!(m.boundary_reject_reasons.is_empty());
        assert!(m.boundary_rejects_detail.is_empty());

        // Staging artifacts exist (the dry run's ONLY artifact surface).
        for f in [
            "machine-tier.ttl",
            "license-restricted-tier.ttl",
            "restricted-public-projection.ttl",
            "combined-batch.json",
            "run-provenance.ttl",
            "batch-quality.ttl",
            "audit-sample.json",
        ] {
            assert!(staging.join(f).exists(), "staging artifact {} missing", f);
        }
        // The batch-quality artifact carries BOTH run-level DQV metrics, computed on the
        // run IRI — the queryable half of the §4.5 "how good was this batch?" surface.
        let quality = std::fs::read_to_string(staging.join("batch-quality.ttl")).unwrap();
        assert_eq!(quality.matches("a dqv:QualityMeasurement").count(), 2);
        assert!(quality.contains(crate::vocab::GROUNDING_RATE_METRIC));
        assert!(quality.contains(crate::vocab::SOURCE_YIELD_RATE_METRIC));
        assert!(quality.contains(&super::pipeline::batch_iri(&run.cfg.run_id)));
        // The run provenance is PROV-O stamped.
        let prov = std::fs::read_to_string(staging.join("run-provenance.ttl")).unwrap();
        assert!(prov.contains("a prov:Activity"));
        assert!(prov.contains("prov:generatedAtTime"));
        assert!(prov.contains("prov:wasAssociatedWith"));

        // Verdict closes the run; the sidecar then carries all three stages.
        run.record_verdict("iterate", "fixture shake-down", "none — harness PR")
            .unwrap();
        let chain = run.sidecar().chain().unwrap();
        let stages: Vec<&str> = chain
            .iter()
            .map(|v| v.get("stage").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(stages, vec!["prereg", "metrics", "verdict"]);
        // The metrics record is honest: verbatim fields + the structural notice.
        let metrics = &chain[1];
        assert_eq!(metrics["grounded"], 4);
        assert_eq!(metrics["boundary_rejects"], 0);
        assert_eq!(metrics["confidence_notice"], UNCALIBRATED_NOTICE);
        assert_eq!(metrics["error"], Value::Null);
        // The committed-sidecar licensing invariant: NO abstract-derived text. Every
        // fixture justification lives in the abstracts; none may appear in the sidecar.
        let sidecar_raw = std::fs::read_to_string(run.sidecar().path()).unwrap();
        assert!(!sidecar_raw.contains("chunk-parallel"), "no abstract text in sidecar");
        assert!(!sidecar_raw.contains("zero-knowledge protocol"), "no abstract text");
    }

    #[test]
    fn boundary_rejects_are_threaded_into_the_sidecar_metrics() {
        // The sq-tx8v9 invariant end-to-end: an extractor's defensive-boundary drops are
        // COUNTED (reason-tagged, no text) in the committed sidecar metrics record — the
        // grounding_rate=1.0/quarantined=0 blind spot the first pilot iteration found.
        let dir = tmpdir("boundary-rejects");
        let run =
            PilotRun::preregister(test_cfg("boundary-rejects"), dir.join("s.jsonl")).unwrap();
        struct Rejecting(RecordedExtractor);
        impl Extractor for Rejecting {
            fn extract(&self, stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String> {
                self.0.extract(stubs)
            }
            fn boundary_rejects(&self) -> Vec<BoundaryReject> {
                vec![
                    BoundaryReject {
                        source_doi: "10.1/a".to_string(),
                        reason: "justification-not-anchored".to_string(),
                    },
                    BoundaryReject {
                        source_doi: "10.1/b".to_string(),
                        reason: "justification-not-anchored".to_string(),
                    },
                    BoundaryReject {
                        source_doi: "10.1/c".to_string(),
                        reason: "unknown-source".to_string(),
                    },
                ]
            }
        }
        let ex = Rejecting(RecordedExtractor::from_fixture().unwrap());
        let mut ledger = CapLedger::new(HardCaps::default());
        let m = run
            .execute_dry(&fixture_inputs(None), &ex, &mut ledger, None)
            .unwrap();
        assert_eq!(m.boundary_rejects, 3, "boundary drops counted, never silent");
        assert_eq!(m.boundary_reject_reasons["justification-not-anchored"], 2);
        assert_eq!(m.boundary_reject_reasons["unknown-source"], 1);
        assert_eq!(m.boundary_rejects_detail.len(), 3);
        // The counts land VERBATIM in the committed sidecar metrics record.
        let chain = run.sidecar().chain().unwrap();
        let metrics = &chain[1];
        assert_eq!(metrics["boundary_rejects"], 3);
        assert_eq!(metrics["boundary_reject_reasons"]["unknown-source"], 1);
        assert_eq!(
            metrics["boundary_rejects_detail"][0]["source_doi"],
            "10.1/a"
        );
        assert_eq!(
            metrics["boundary_rejects_detail"][0]["reason"],
            "justification-not-anchored"
        );
        // No abstract-derived text rides along (DOIs + categories only).
        let raw = std::fs::read_to_string(run.sidecar().path()).unwrap();
        assert!(!raw.contains("chunk-parallel"), "no abstract text in sidecar");
    }

    #[test]
    fn a_failed_run_still_records_an_error_metrics_trace() {
        let dir = tmpdir("err-trace");
        let run = PilotRun::preregister(test_cfg("err-trace"), dir.join("s.jsonl")).unwrap();
        struct Failing;
        impl Extractor for Failing {
            fn extract(&self, _stubs: &[SourceStub]) -> Result<Vec<CandidateFinding>, String> {
                Err("sub-agent died".to_string())
            }
        }
        let mut ledger = CapLedger::new(HardCaps::default());
        let err = run
            .execute_dry(&fixture_inputs(None), &Failing, &mut ledger, None)
            .unwrap_err();
        assert!(err.contains("sub-agent died"));
        let chain = run.sidecar().chain().unwrap();
        assert_eq!(chain.len(), 2, "prereg + error metrics");
        assert!(chain[1]["error"]
            .as_str()
            .unwrap()
            .contains("sub-agent died"));
    }

    // ---- Findings enumeration + sampling ---------------------------------------------

    #[test]
    fn list_findings_reads_the_emitted_artifact() {
        let ex = RecordedExtractor::from_fixture().unwrap();
        let out = pipeline::run_tiered(
            FIXTURE_OPENALEX_BATCH,
            &ex,
            Some("2026-07-05T12:00:00Z".to_string()),
            BatchCompleteness::Complete,
        )
        .unwrap();
        let restricted = list_findings(&out.license_restricted_tier);
        assert_eq!(restricted.len(), 4, "all four grounded findings enumerate");
        for f in &restricted {
            assert!(f.finding_iri.contains("/finding/"));
            assert!(f.source_iri.starts_with("https://doi.org/"));
            assert!(!f.justification.is_empty());
        }
        assert!(list_findings(&out.machine_tier).is_empty());
    }

    #[test]
    fn uniform_sample_is_deterministic_and_bounded() {
        let findings: Vec<FindingRef> = (0..50)
            .map(|i| FindingRef {
                finding_iri: format!("f{}", i),
                source_iri: format!("s{}", i),
                justification: String::new(),
            })
            .collect();
        let a = select_uniform_sample(&findings, 20, 42);
        let b = select_uniform_sample(&findings, 20, 42);
        assert_eq!(a, b, "same seed => same sample");
        assert_eq!(a.len(), 20);
        let distinct: std::collections::HashSet<&str> =
            a.iter().map(|f| f.finding_iri.as_str()).collect();
        assert_eq!(distinct.len(), 20, "sampling without replacement");
        // Fewer findings than target: all returned (the audit abstains upstream).
        assert_eq!(select_uniform_sample(&findings[..7], 20, 42).len(), 7);
        // A different seed reorders (probabilistically certain at n=50; fixed seeds).
        let c = select_uniform_sample(&findings, 20, 43);
        assert_ne!(a, c);
    }

    // ---- Seed registry + OpenAlex normalisation ---------------------------------------

    #[test]
    fn parse_seed_registry_reads_the_committed_registry() {
        let raw = include_str!("../../ingest/literature-seeds.toml");
        let seeds = parse_seed_registry(raw).unwrap();
        assert!(seeds.len() >= 20, "the committed registry has its seeds");
        let ids: std::collections::HashSet<&str> =
            seeds.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), seeds.len(), "ids unique");
        let neuro = seeds
            .iter()
            .find(|s| s.id == "neurosymbolic-knowledge-integration")
            .expect("the neurosymbolic seed exists");
        assert_eq!(neuro.topic, "neurosymbolic AI");
        assert!(neuro.max_records > 0);
    }

    #[test]
    fn openalex_live_records_normalise_inverted_abstract_and_license() {
        let page = r#"{ "meta": { "count": 2 }, "results": [
            { "doi": "https://doi.org/10.1/A", "display_name": "Inverted",
              "publication_year": 2024,
              "abstract_inverted_index": { "graphs": [1], "Knowledge": [0], "win": [2] },
              "primary_location": { "license": "cc-by" } },
            { "doi": "10.1/b", "title": "Plain", "abstract": "already plain",
              "best_oa_location": { "license": "cc-by-nc" } }
        ] }"#;
        let records = normalise_openalex_records(page).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["abstract"], "Knowledge graphs win");
        assert_eq!(records[0]["title"], "Inverted");
        assert_eq!(records[0]["license"], "cc-by");
        assert_eq!(records[1]["abstract"], "already plain");
        assert_eq!(records[1]["license"], "cc-by-nc");
        // The combined batch feeds the existing connector parse unchanged.
        let batch = combine_records(&records);
        let (stubs, skipped) = parse_openalex_batch(&batch).unwrap();
        assert_eq!(stubs.len(), 2);
        assert_eq!(skipped, 0);
        assert_eq!(stubs[0].doi, "10.1/a");
        assert_eq!(stubs[0].abstract_text, "Knowledge graphs win");
        assert_eq!(stubs[0].license.as_deref(), Some("cc-by"));
    }

    #[test]
    fn records_from_stubs_round_trip_through_the_connector() {
        let stubs = vec![SourceStub {
            doi: "10.9/x".to_string(),
            title: "T".to_string(),
            abstract_text: "abs".to_string(),
            year: Some(2020),
            license: None,
        }];
        let batch = combine_records(&records_from_stubs(&stubs));
        let (parsed, skipped) = parse_openalex_batch(&batch).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(parsed, stubs);
    }

    #[test]
    fn reason_categories_are_stable() {
        assert_eq!(
            categorize_reason("justification is not a span of the source abstract"),
            "justification-not-anchored"
        );
        assert_eq!(
            categorize_reason("cited DOI does not resolve to an in-batch Source: 10.1/x"),
            "dangling-citation"
        );
        assert_eq!(categorize_reason("source DOI not in batch"), "unknown-source");
        assert_eq!(categorize_reason("???"), "other");
    }

    #[test]
    fn today_utc_is_a_date_prefix() {
        let t = today_utc();
        assert_eq!(t.len(), 10);
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[7..8], "-");
    }

    // ---- The COMMITTED first-iteration sidecar (auditable loop history) ---------------

    /// The first real iteration's sidecar (live OpenAlex fetch + live Haiku extraction,
    /// 2026-07-06, dry run — strict audited precision 0.65 < the 0.8 pre-registered bar,
    /// so the verdict is `iterate` and no KB write happened). Committed VERBATIM; this
    /// test makes any retro-edit a CI failure — the append-only invariant for history too.
    const COMMITTED_RUN_1: &str = include_str!(
        "../../fixtures/literature/pilot-runs/neurosymbolic-knowledge-integration-20260706011507303323.sidecar.jsonl"
    );

    #[test]
    fn committed_first_iteration_sidecar_verifies_and_stays_gated() {
        let chain = verify_chain(COMMITTED_RUN_1).expect("committed sidecar chain verifies");
        let stages: Vec<&str> = chain
            .iter()
            .map(|v| v.get("stage").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(stages, vec!["prereg", "metrics", "audit", "verdict"]);
        // The recorded audit did NOT clear the pre-registered bar (0.65 < 0.8) …
        let audit = &chain[2];
        assert_eq!(audit["passed_bar"], false);
        assert_eq!(audit["audited_n"], 20);
        // … so the live-emit gate REFUSES on this sidecar, forever.
        let err = live_emit_allowed(COMMITTED_RUN_1).unwrap_err();
        assert!(err.contains("below the pre-registered bar"), "got: {}", err);
        // The verdict is a recorded `iterate` with the next-iteration change noted.
        assert_eq!(chain[3]["verdict"], "iterate");
        assert!(chain[3]["changes_before_next_iteration"]
            .as_str()
            .unwrap()
            .contains("tighten"));
        // The run was a dry run under the default (un-overridden) bar + caps.
        assert_eq!(chain[0]["dry_run"], true);
        assert_eq!(chain[0]["bar"]["overridden"], false);
        // Honest verbatim metrics from the real run (counts, not claims).
        let m = &chain[1];
        assert_eq!(m["fetched_records"], 24);
        assert_eq!(m["shacl_checked"], true);
        assert_eq!(m["error"], Value::Null);
        // The structural UNCALIBRATED notice is on both prereg and metrics.
        assert_eq!(chain[0]["confidence_notice"], UNCALIBRATED_NOTICE);
        assert_eq!(chain[1]["confidence_notice"], UNCALIBRATED_NOTICE);
    }
}
