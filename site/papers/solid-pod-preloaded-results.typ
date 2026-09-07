// [GPT-6] Native results only; this helper loads no data and has no cached fallback.
#let object(value) = if type(value) == dictionary { value } else { (:) }
#let native-result-state(data) = {
  if data == none { return "unmeasured" }
  let data = object(data)
  if data.at("schema_version", default: none) != 1 or data.at("analysis_kind", default: "") != "native-preloaded-campaign-independent-accounting" {
    return "incompatible native analysis"
  }
  let review = object(data.at("source_review", default: none))
  if review.at("status", default: "") == "quarantined" or review.at("quarantine_events", default: ()).len() > 0 or data.at("events", default: ()).any(row => object(row).at("record_type", default: "") == "correctness-quarantine") {
    return "quarantined"
  }
  if data.at("execution_status", default: "") != "complete" or data.at("cells", default: ()).any(cell => cell.at("execution_status", default: "") not in ("complete", "admission-failed")) { return "incomplete" }
  let integrity = object(data.at("artifact_integrity", default: none))
  if integrity.at("complete", default: false) != true or integrity.at("manifest_status", default: "") != "verified" or integrity.at("errors", default: ()).len() > 0 or integrity.at("parsing_errors", default: ()).len() > 0 {
    return "invalid integrity"
  }
  let verified = object(data.at("source_input_verification", default: none))
  if verified.at("passed", default: false) != true or verified.at("issues", default: ()).len() > 0 or data.at("global_issues", default: ()).len() > 0 {
    return "unverified evidence"
  }
  let record = object(review.at("record", default: none))
  let manifest = object(integrity.at("parsed_input_sha256", default: none)).at("MANIFEST.sha256", default: none)
  if review.at("status", default: "") != "passed" or record.at("status", default: "") != "passed" or review.at("matches_exact_result", default: false) != true or manifest == none or record.at("manifest_sha256", default: none) != manifest {
    return "review mismatch"
  }
  let campaign = object(data.at("campaign", default: none))
  for field in ("source_commit", "binary_sha256", "campaign_sha256") {
    let value = data.at(field, default: none)
    if value == none or record.at(field, default: none) != value { return "review mismatch" }
  }
  if data.at("binary_build_source_commit", default: none) != data.source_commit or campaign.at("source_commit", default: none) != data.source_commit or campaign.at("binary_build_source_commit", default: none) != data.source_commit or campaign.at("binary_sha256", default: none) != data.binary_sha256 or campaign.at("campaign_id", default: none) != data.at("campaign_id", default: none) {
    return "source mismatch"
  }
  "finalized and reviewed"
}

#let native-response-rows(data, labels: none) = {
  if native-result-state(data) != "finalized and reviewed" { return () }
  data.at("cells", default: ()).filter(cell => labels == none or cell.label in labels).map(cell => {
    let timed = cell.at("execution_status", default: "") == "complete" and cell.at("valid_for_inference", default: false) == true and object(cell.at("preload", default: none)).at("passed", default: false) == true
    let admission-failed = cell.at("execution_status", default: "") == "admission-failed" and cell.at("valid_for_admission_inference", default: false) == true and cell.at("requests", default: none) == none
    let requests = if timed { object(cell.at("requests", default: none)) } else { (:) }
    let latency = object(object(requests.at("latency_us", default: none)).at("successful:scheduled_latency_us", default: none))
    (
      label: cell.label, dataset: cell.dataset, model: cell.model,
      population: cell.population, memory_gib: cell.memory_gib, cpus: cell.cpus,
      state: if timed { cell.local_guard } else if admission-failed { "admission-failed" } else { "unmeasured or inconclusive" },
      successful_p95_us: latency.at("p95", default: none),
      success_fraction_of_offered: requests.at("success_fraction_of_offered", default: none),
      deadline_fraction_of_offered: requests.at("deadline_fraction_of_offered", default: none),
    )
  })
}

#let native-history-admission(data) = {
  if native-result-state(data) != "finalized and reviewed" { return none }
  data.at("retained_history_million_pod_admission", default: none)
}

#let native-response-table(data, labels: none) = {
  let state = native-result-state(data)
  if state != "finalized and reviewed" { return [Native results: #state.] }
  let display(value, scale: 1) = if value == none { "—" } else if scale == 1 { str(value) } else { str(calc.round(value * scale, digits: 3)) }
  table(columns: (2fr, 1fr, 1.5fr, 1fr, 1fr, 1fr),
    table.header([*Dataset / model*], [*RAM GiB*], [*Result*], [*p95 ms*], [*Successful fraction*], [*Timely fraction*]),
    ..native-response-rows(data, labels: labels).map(row => (
      [#row.dataset / #row.model], [#row.memory_gib], [#row.state],
      [#display(row.successful_p95_us, scale: 0.001)],
      [#display(row.success_fraction_of_offered)], [#display(row.deadline_fraction_of_offered)],
    )).flatten(),
  )
}
