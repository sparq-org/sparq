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
// Per-run summaries remain separate. Invalid/inconclusive records supply no
// apparent zero, while a valid all-failure run retains its measured zero fractions.
#let observed-range(values) = {
  let present = values.filter(v => v != none and (type(v) == int or type(v) == float) and v >= 0)
  (n: present.len(), lower: if present.len() > 0 { calc.min(..present) } else { none },
    upper: if present.len() > 0 { calc.max(..present) } else { none })
}
#let model-responses(cells, model) = {
  let planned = cells.filter(c => c.model == model)
  let valid = planned.filter(c => c.at("valid_for_inference", default: false) and
    ("pass", "fail").contains(c.local_guard) and c.at("requests", default: none) != none)
  let p95 = valid.map(c => c.requests.at("latency_us", default: (:)).at("successful:scheduled_latency_us", default: (:)).at("p95", default: none))
  let timely = valid.map(c => c.requests.at("deadline_fraction_of_offered", default: none))
  let successful = valid.map(c => c.requests.at("success_fraction_of_offered", default: none))
  (planned_runs: planned.len(), valid_runs: valid.len(),
    successful_p95_us: observed-range(p95), timely_fraction: observed-range(timely),
    success_fraction: observed-range(successful))
}
#let range-label(observation, scale: 1, digits: 1) = {
  if observation.n == 0 { return "—" }
  // Outward rounding prevents nearly complete service fractions displaying 100%.
  let precision = calc.pow(10, digits)
  let lo = calc.floor(observation.lower * (scale * precision)) / precision
  let hi = calc.ceil(observation.upper * (scale * precision)) / precision
  if lo == hi { str(lo) } else { str(lo) + "–" + str(hi) }
}
#let range-cell(observation, scale: 1, digits: 1) = [#range-label(observation, scale: scale, digits: digits)#if observation.n > 0 { super(str(observation.n)) }]

#let offered-label(cells, pair) = {
  if pair.rate_override != "derived" { return pair.rate_override }
  let rates = cells.filter(c => c.valid_for_inference and c.at("requests", default: none) != none).map(c => c.requests.offered_rate)
  if rates.len() == 0 { return "derived; unavailable" }
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
    set text(size: 8pt)
    table(columns: (1.8fr, 0.72fr, 0.85fr, 0.95fr, 0.72fr, 0.85fr, 0.95fr, 1.03fr), inset: 3pt,
      table.header(
        table.cell(rowspan: 2)[*Corpus / lane*\ *CPU/GiB · rps*],
        table.cell(colspan: 3)[*WAC*], table.cell(colspan: 3)[*ACP*],
        table.cell(rowspan: 2)[*p95 ratio*\ *95% CI*],
        [*P/F/I/U*], [*OK p95*\ *ms*], [*Timely %*\ *OK %*],
        [*P/F/I/U*], [*OK p95*\ *ms*], [*Timely %*\ *OK %*],
      ),
      ..data.paired_comparisons.map(p => {
        let rows = paired-cells(data, p)
        let ci = p.paired_p95_scheduled_response_ratio
        let wac = model-responses(rows, "wac")
        let acp = model-responses(rows, "acp")
        (
          [#dataset-label(p.dataset)\ #text(size: 7pt)[#group-label(p.group)\ #p.cpus/#p.memory_gib · #offered-label(rows, p)]],
          [#verdict-counts(rows, "wac")],
          [#range-cell(wac.successful_p95_us, scale: 0.001)],
          [#range-cell(wac.timely_fraction, scale: 100, digits: 2)\ #range-cell(wac.success_fraction, scale: 100, digits: 2)],
          [#verdict-counts(rows, "acp")],
          [#range-cell(acp.successful_p95_us, scale: 0.001)],
          [#range-cell(acp.timely_fraction, scale: 100, digits: 2)\ #range-cell(acp.success_fraction, scale: 100, digits: 2)],
          [#if ci.available { bounds(ci.ci95) } else { "unavailable" }],
        )
      }).flatten(),
    )
  }, caption: [Main local HTTP cells. P/F/I/U count passing, valid failing,
    inconclusive and unmeasured repetitions. Absolute columns show between-run
    min–max ranges, rounded outward; superscripts count contributing valid runs.
    A dash is unavailable, not zero. “OK p95” uses successful responses only,
    from scheduled arrival to complete body. “Timely” and “OK” divide successful
    responses within #data.campaign.measurement.server_deadline_ms ms and all
    successful responses by *all offered requests*, including
    errors and drops. The ACP/WAC p95 CI resamples matched runs; it neither pools
    requests nor establishes a service pass by itself.])

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
