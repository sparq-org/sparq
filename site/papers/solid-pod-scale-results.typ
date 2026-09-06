// [GPT-6] Optional presentation of source-bound main campaign analysis.
// No supplied analysis, partial artifacts, or quarantined sources can emit results.
#let main-state(data) = {
  if data == none { return "absent (campaign in progress)" }
  if data.at("schema_version", default: none) != 1 or data.at("analysis_kind", default: "") != "frozen-campaign-independent-accounting" {
    return "incompatible analysis schema"
  }
  let review = data.at("source_review", default: (:))
  if review.at("status", default: "unreviewed") == "quarantined" or review.at("quarantine_events", default: ()).len() > 0 {
    return "source quarantined; no main inference"
  }
  let integrity = data.at("artifact_integrity", default: (:))
  if not integrity.at("complete", default: false) or integrity.at("manifest_status", default: "") != "verified" or integrity.at("errors", default: ()).len() > 0 {
    return "incomplete or checksum-invalid"
  }
  if review.at("status", default: "unreviewed") != "passed" or not review.at("source_binding_matches", default: false) {
    return "source review absent or mismatched"
  }
  "finalized and reviewed"
}

#let num(value, digits: 1) = if value == none { "—" } else { str(calc.round(value, digits: digits)) }
#let units(value, divisor) = num(if value == none { none } else { value / divisor })
#let bounds(interval) = if interval == none { "unestablished" } else {
  // Outward rounding preserves the printed uncertainty interval.
  "[" + str(calc.floor(interval.first() * 1000) / 1000) + ", " + str(calc.ceil(interval.last() * 1000) / 1000) + "]"
}
#let paired-cells(data, pair) = data.cells.filter(c =>
  c.dataset == pair.dataset and c.group == pair.group and
  c.memory_gib == pair.memory_gib and c.cpus == pair.cpus and
  str(c.rate_override) == pair.rate_override)
#let verdict-counts(cells, model) = {
  let rows = cells.filter(c => c.model == model)
  ("pass", "fail", "inconclusive", "unmeasured").map(state => str(rows.filter(c => c.local_guard == state).len())).join("/")
}
#let offered-label(cells, pair) = {
  if pair.rate_override != "derived" { return pair.rate_override }
  let rates = cells.filter(c => c.valid_for_inference and c.at("requests", default: none) != none).map(c => c.requests.offered_rate)
  if rates.len() == 0 { return "derived; unmeasured" }
  let lo = calc.min(..rates)
  let hi = calc.max(..rates)
  if lo == hi { num(lo) } else { num(lo) + "–" + num(hi) }
}
#let dataset-label(dataset) = dataset.replace("-", " ")
#let group-label(group) = (
  "compact-fixed-population": "fixed load",
  "population-control": "derived demand",
  "compact-rate-bracket": "rate search",
  "retained-history-cold": "cold history",
  "retained-history-larger": "history diagnosis",
  "retained-history-hot": "hot history",
).at(group, default: group)

#let main-tables(data) = {
  assert(main-state(data) == "finalized and reviewed", message: "Main tables require finalized, source-reviewed analysis")
  figure({
    set text(size: 8.5pt)
    table(columns: (1.5fr, 0.55fr, 0.68fr, 0.8fr, 0.8fr, 0.9fr), inset: 4pt,
      table.header([*Corpus / lane*], [*CPU / GiB*], [*Offered rps*], [*WAC*\ *P/F/I/U*], [*ACP*\ *P/F/I/U*], [*p95 ratio*\ *95% CI*]),
      ..data.paired_comparisons.map(p => {
        let rows = paired-cells(data, p)
        let ci = p.paired_p95_scheduled_response_ratio
        (
          [#dataset-label(p.dataset)\ #text(size: 7.5pt)[#group-label(p.group)]],
          [#p.cpus / #p.memory_gib], [#offered-label(rows, p)],
          [#verdict-counts(rows, "wac")], [#verdict-counts(rows, "acp")],
          [#if ci.available { bounds(ci.ci95) } else { "unavailable" }],
        )
      }).flatten(),
    )
  }, caption: [Main local HTTP cells. Counts retain every planned repetition:
    P pass, F valid failure, I inconclusive, U unmeasured. The p95 ACP/WAC ratio
    resamples matched independent runs and concerns successful scheduled-arrival
    to complete-body responses. A ratio alone does not establish a service pass;
    admission retains failures in every offered-request denominator.])

  let brackets = data.capacity_brackets
  if brackets.len() > 0 {
    figure({
      set text(size: 8.5pt)
      table(columns: (1.35fr, 0.8fr, 0.8fr, 1fr, 1fr), inset: 4pt,
        table.header([*Corpus / tier*], [*WAC pass / fail rps*], [*ACP pass / fail rps*], [*Capacity-ratio bounds*], [*Capacity equivalence*]),
        ..brackets.map(b => (
          [#dataset-label(b.dataset)\ #b.cpus CPU / #b.memory_gib GiB],
          [#num(b.models.wac.highest_tested_passing_rate) / #num(b.models.wac.lowest_consistently_failing_rate)],
          [#num(b.models.acp.highest_tested_passing_rate) / #num(b.models.acp.lowest_consistently_failing_rate)],
          [#bounds(b.conservative_capacity_ratio_bounds)],
          [#if b.capacity_equivalence == "bounded-within-margin" { "bounded within margin" } else { "unestablished" }],
        )).flatten(),
      )
    }, caption: [Operational capacity bounds under the frozen monotonic-capacity
      assumption. A missing bound is shown as a dash. Coarse tested rates leave
      untested intervals; neither matching passing grid points nor their bootstrap
      interval establishes capacity equivalence. Displayed interval endpoints are
      rounded outward; decisions use full precision.])
  }

  let indexed = data.at("indexed_component_diagnostics", default: ()).filter(d => d.valid_for_component_inference)
  if indexed.len() > 0 {
    figure({
      set text(size: 8.5pt)
      table(columns: (0.5fr, 0.75fr, 1fr, 1fr, 0.75fr, 1fr), inset: 4pt,
        table.header([*Policy*], [*Index form*], [*Parse + ready*\ *median ms*], [*Open + ready*\ *median ms*], [*Files*], [*Allocated MiB*]),
        ..indexed.map(d => ("raw", "compressed").map(storage => (
          [#upper(d.model)], [#storage],
          [#units(d.timing_ns.parse_plus_memory_authorized_ready_ns.p50, 1000000)],
          [#units(d.timing_ns.at("open:" + storage + ":open_to_authorized_ready_ns").p50, 1000000)],
          [#d.footprint.at(storage).final_list_files],
          [#units(d.footprint.at(storage).snapshots.open.allocated_bytes.total, calc.pow(2, 20))],
        ))).flatten().flatten(),
      )
    }, caption: [Bounded indexed component diagnostic:
      #indexed.map(d => upper(d.model) + " " + str(d.configuration.pods) + " Pods / " + str(d.exact_comparisons.confirmed_matches) + " exact comparisons").join("; ").
      Timings include authorization readiness; parsing excludes generation and save.
      Reopening retains index validation. Medians span these Pods, not independent
      repetitions. Cache state is uncontrolled after file generation; file and
      allocated-byte totals cover each stored index form. No HTTP or million-Pod
      indexed-capacity result follows.])
  }

  [The source-bound analysis retains cache hit, miss and missing-header counts
  alongside phase distributions; missing headers are never inferred misses.
  Per-operation failures, resolved mutations, no-ops, policy probes, inventories
  and cgroup snapshots remain available for every measured cell. The full table
  also records why each planned cell failed or remained unmeasured.]
}
