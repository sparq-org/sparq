// Canonical wall-clock/process-CPU evidence accessors.
//
// Deliberately separate from bench.typ. There is no raw-value or raw-image-path accessor:
// every rendered timing value and every analyzer-generated timing SVG carries run provenance
// in the same Typst content node. build-papers.mjs supplies the generated merged-ledger path as
// `data` and the current slug as `paper`, so a record also cannot be reused outside its declared
// paper set. Passing a path (rather than the whole JSON as one CLI argument) avoids Linux's
// per-argument size limit for dense studies.

#let _evidence = json(sys.inputs.data)

#let _timing_rec(key, expected_kind: none) = {
  let r = _evidence.records.at(key, default: none)
  if r == none {
    panic("paper-factory: unknown canonical timing key '" + key + "'")
  }
  if r.environment != "canonical-timing" {
    panic(
      "paper-factory: timing accessor for '" + key + "' requires " +
      "environment='canonical-timing'; found '" + r.environment + "'",
    )
  }
  if expected_kind != none and r.kind != expected_kind {
    panic(
      "paper-factory: timing accessor for '" + key + "' requires kind='" +
      expected_kind + "'; found '" + r.kind + "'",
    )
  }
  if r.at("_generated_by", default: none) != "site/scripts/sync-canonical-timing.mjs" {
    panic("paper-factory: canonical timing key '" + key + "' was not sync-generated")
  }
  let paper = sys.inputs.at("paper", default: none)
  let papers = r.at("papers", default: ())
  if paper == none or not (paper in papers) {
    panic(
      "paper-factory: canonical timing key '" + key + "' is not declared for paper '" +
      str(paper) + "'",
    )
  }
  let p = r.at("timing_provenance", default: none)
  if p == none {
    panic("paper-factory: canonical timing key '" + key + "' has no timing_provenance")
  }
  if r.kind != "canonical-timing-figure" {
    let bindings = r.at("result_bindings", default: none)
    if type(bindings) != array or bindings.len() == 0 {
      panic("paper-factory: canonical timing key '" + key + "' has no source-artifact bindings")
    }
    for binding in bindings {
      for field in ("artifact", "sha256", "locator") {
        if binding.at(field, default: none) == none {
          panic(
            "paper-factory: canonical timing key '" + key +
            "' has an incomplete source-artifact binding",
          )
        }
      }
    }
  }
  for field in (
    "study_id",
    "run_id",
    "collected_at_utc",
    "source_git_commit",
    "analysis_git_commit",
    "host_class",
    "host_label",
    "tenancy",
    "noise_limitation",
    "workload",
    "dataset",
    "query_scope",
    "protocol",
    "bootstrap_draws",
    "bootstrap_seed",
    "publisher_path",
    "publisher_sha256",
    "input_file_count",
    "raw_archive_kind",
    "raw_archive_location",
    "raw_archive_public_url",
    "raw_archive_sha256",
    "raw_archive_bytes",
    "raw_archive_build_verification",
    "raw_archive_member_verification_authority",
    "raw_archive_manifest_member",
    "raw_archive_manifest_sha256",
    "raw_archive_manifest_bytes",
    "raw_archive_manifest_entry_count",
    "raw_archive_regular_members",
    "raw_archive_all_members_rehashed",
    "raw_archive_exact_member_set",
    "raw_archive_sanitization_scan_passed",
    "raw_archive_deterministic_tar_headers",
    "raw_archive_zstd_version",
    "raw_archive_zstd_executable_sha256",
  ) {
    if p.at(field, default: none) == none {
      panic("paper-factory: canonical timing key '" + key + "' lacks provenance field '" + field + "'")
    }
  }
  r
}

#let _render_number(value, digits) = {
  if digits == none { str(value) } else { str(calc.round(value, digits: digits)) }
}

#let _full_provenance_text(r, include_digest: false) = {
  let p = r.timing_provenance
  [
    Canonical timing evidence: #p.workload; dataset #p.dataset; query scope #p.query_scope;
    run #raw(p.run_id); #p.host_class, #p.host_label; tenancy #p.tenancy.
    Host limitation: #p.noise_limitation
    collected #p.collected_at_utc; benchmark/source commit #raw(p.source_git_commit);
    analysis commit #raw(p.analysis_git_commit); protocol
    #raw(p.protocol); cluster bootstrap #p.bootstrap_draws draws, seed #p.bootstrap_seed.
    Bound source: #raw(r.source).
    #if r.kind != "canonical-timing-figure" [
      Derived source binding(s): #for binding in r.result_bindings [
        #raw(binding.artifact) (SHA-256 #raw(binding.sha256), locator
        #raw(binding.locator));
      ]
    ]
    #if include_digest [
      Analyzer output #raw(r.figure.output_name) (#r.figure.bytes bytes); SVG SHA-256:
      #raw(r.value).
    ]
    Raw input archive: #raw(p.raw_archive_location), SHA-256 #raw(p.raw_archive_sha256)
    (#p.raw_archive_bytes bytes). #if p.raw_archive_public_url != "" [
      Public mirror: #raw(p.raw_archive_public_url).
    ] #if p.raw_archive_build_verification == "local-rehash" [
      The sync/build independently re-hashes the committed archive container and checks its byte
      length.
    ] else [
      This is an external descriptor-only fallback: the paper build does not fetch or re-hash
      the remote archive, so reproduction must verify the displayed digest after download.
    ]
    Publisher-recorded member verification: the locally re-hashed publisher
    #raw(p.publisher_path) (SHA-256 #raw(p.publisher_sha256)) records that all
    #p.raw_archive_regular_members regular members were re-hashed, their set exactly matched
    manifest #raw(p.raw_archive_manifest_member) (SHA-256
    #raw(p.raw_archive_manifest_sha256), #p.raw_archive_manifest_bytes bytes,
    #p.raw_archive_manifest_entry_count entries), the sanitization scan passed, and tar headers
    were deterministic. #p.input_file_count analysis input descriptor(s) name exact archive
    members. Compression: zstd #raw(p.raw_archive_zstd_version), executable SHA-256
    #raw(p.raw_archive_zstd_executable_sha256). These member-level facts are an attestation
    recorded by the publisher; sync/build validates the attestation's shape and publisher bytes
    but does not independently unpack and re-hash archive members.
  ]
}

// Inline values carry a compact, record-specific provenance note. Printing every raw
// artifact/hash/locator triple in every footnote is both redundant and pathological for
// aggregate correctness records (hundreds of valid bindings can turn one sentence into many
// pages). The complete bindings remain mandatory in `_timing_rec`, in the generated ledger,
// and in the immutable envelope; `timing_provenance` renders the full acquisition/archive
// statement once in the manuscript's artifact section.
#let _provenance_text(r, include_digest: false) = {
  let p = r.timing_provenance
  let source_parts = r.source.split("#")
  let pointer = if source_parts.len() > 1 { source_parts.last() } else { r.source }
  let bindings = if r.kind == "canonical-timing-figure" { 0 } else { r.result_bindings.len() }
  [
    Canonical #raw(p.run_id): #raw(pointer);
    #if r.kind != "canonical-timing-figure" [
      #bindings bound source(s);
    ]
    #if include_digest [
      #raw(r.figure.output_name) SHA-256 #raw(r.value);
    ]
    see artifact statement.
  ]
}

// The only public timing-value accessor. A caller may choose rounding and a display suffix,
// but cannot render the number without the run provenance footnote created in this node.
#let headline_timing(key, digits: none, suffix: none) = {
  let r = _timing_rec(key, expected_kind: "canonical-timing")
  if type(r.value) != int and type(r.value) != float {
    panic("paper-factory: canonical timing value '" + key + "' must be numeric")
  }
  let rendered = _render_number(r.value, digits)
  box[
    #rendered#if suffix != none [#suffix]#footnote(_provenance_text(r))
  ]
}

// Timing-derived prose must be represented as a boolean decision, never as an unchecked
// free-form string in the evidence ledger. Both branches remain manuscript text (and therefore
// pass through the prose honesty gates); the selected branch and provenance are inseparable.
#let timing_verdict(key, yes: [meets], no: [does not meet]) = {
  let r = _timing_rec(key, expected_kind: "canonical-timing-verdict")
  if type(r.value) != bool {
    panic("paper-factory: canonical timing verdict '" + key + "' must be boolean")
  }
  if not (r.at("hypothesis", default: none) in ("H1", "H2")) {
    panic("paper-factory: only H1/H2 may render as mechanical timing verdicts")
  }
  box[
    #if r.value { yes } else { no }#footnote(_provenance_text(r))
  ]
}

#let _timing_fingerprint(r) = {
  let p = r.timing_provenance
  (
    p.study_id,
    p.run_id,
    p.collected_at_utc,
    p.source_git_commit,
    p.analysis_git_commit,
    p.host_class,
    p.host_label,
    p.tenancy,
    p.noise_limitation,
    p.workload,
    p.dataset,
    p.query_scope,
    p.protocol,
    p.bootstrap_draws,
    p.bootstrap_seed,
    p.publisher_path,
    p.publisher_sha256,
    p.input_file_count,
    p.raw_archive_kind,
    p.raw_archive_location,
    p.raw_archive_public_url,
    p.raw_archive_sha256,
    p.raw_archive_bytes,
    p.raw_archive_build_verification,
    p.raw_archive_member_verification_authority,
    p.raw_archive_manifest_member,
    p.raw_archive_manifest_sha256,
    p.raw_archive_manifest_bytes,
    p.raw_archive_manifest_entry_count,
    p.raw_archive_regular_members,
    p.raw_archive_all_members_rehashed,
    p.raw_archive_exact_member_set,
    p.raw_archive_sanitization_scan_passed,
    p.raw_archive_deterministic_tar_headers,
    p.raw_archive_zstd_version,
    p.raw_archive_zstd_executable_sha256,
  )
}

#let _table_spec_record(spec) = {
  let key = if type(spec) == str {
    spec
  } else if type(spec) == dictionary {
    spec.at("key", default: none)
  } else {
    none
  }
  if key == none {
    panic("paper-factory: every timing-table result cell must be a key or (key: ...) dictionary")
  }
  let r = _timing_rec(key)
  if r.kind == "canonical-timing-figure" {
    panic("paper-factory: timing SVG key '" + key + "' cannot be used as a table cell")
  }
  r
}

#let _render_table_spec(spec) = {
  let r = _table_spec_record(spec)
  if r.kind == "canonical-timing" {
    let digits = if type(spec) == dictionary { spec.at("digits", default: none) } else { none }
    let suffix = if type(spec) == dictionary { spec.at("suffix", default: none) } else { none }
    let rendered = _render_number(r.value, digits)
    [#rendered#if suffix != none [#suffix]]
  } else if r.kind == "canonical-timing-verdict" {
    let yes = if type(spec) == dictionary { spec.at("yes", default: [meets]) } else { [meets] }
    let no = if type(spec) == dictionary { spec.at("no", default: [does not meet]) } else { [does not meet] }
    if r.value { yes } else { no }
  } else {
    panic("paper-factory: unsupported timing-table record kind '" + r.kind + "'")
  }
}

// Dense result tables use one provenance footnote for a homogeneous acquisition run rather
// than repeating the same note in every numeric cell. The first `label_columns` cells in each
// row are author-written labels; every remaining cell MUST be an evidence-key descriptor.
// Mixing runs/commits/protocols in one table fails closed.
#let timing_table(
  anchor,
  columns: auto,
  header: none,
  rows: none,
  label_columns: 1,
  caption: none,
) = {
  let anchor_record = _timing_rec(anchor)
  if anchor_record.kind == "canonical-timing-figure" {
    panic("paper-factory: timing_table anchor cannot be a figure record")
  }
  if type(header) != array or type(rows) != array or caption == none {
    panic("paper-factory: timing_table requires header, rows, and caption")
  }
  if label_columns < 0 {
    panic("paper-factory: timing_table label_columns cannot be negative")
  }
  for row in rows {
    if type(row) != array or row.len() <= label_columns {
      panic("paper-factory: every timing_table row needs labels plus at least one result cell")
    }
    if row.len() != header.len() {
      panic("paper-factory: timing_table row/header width mismatch")
    }
  }
  let records = rows
    .map(row => row.slice(label_columns).map(_table_spec_record))
    .flatten()
  let fingerprint = _timing_fingerprint(anchor_record)
  if not records.all(r => _timing_fingerprint(r) == fingerprint) {
    panic("paper-factory: one timing_table cannot mix acquisition runs, commits, or protocols")
  }
  let cells = rows
    .map(row => row.slice(0, label_columns) + row.slice(label_columns).map(_render_table_spec))
    .flatten()
  figure(
    table(columns: columns, table.header(..header), ..cells),
    caption: [#caption #footnote(_provenance_text(anchor_record))],
  )
}

// The only public accessor for analyzer-generated timing graphics. The sync has already
// recomputed the SVG digest and the evidence verifier has checked value == envelope hash.
// Keeping path lookup inside this helper makes the figure and its provenance inseparable.
#let timing_figure(key, width: 100%, caption: none) = {
  let r = _timing_rec(key, expected_kind: "canonical-timing-figure")
  let f = r.at("figure", default: none)
  if f == none or f.at("media_type", default: none) != "image/svg+xml" {
    panic("paper-factory: canonical timing figure '" + key + "' lacks an SVG descriptor")
  }
  let path = f.at("typst_path", default: none)
  let alt = f.at("alt", default: none)
  if path == none or alt == none {
    panic("paper-factory: canonical timing figure '" + key + "' lacks path/alt text")
  }
  let rendered_caption = if caption == none { [Canonical timing figure.] } else { caption }
  figure(
    image(path, width: width, alt: alt),
    caption: [#rendered_caption #footnote(_provenance_text(r, include_digest: true))],
  )
}

// A non-numeric block for an artifact/reproducibility section. It never exposes r.value.
#let timing_provenance(key) = {
  let r = _timing_rec(key)
  _full_provenance_text(r, include_digest: r.kind == "canonical-timing-figure")
}
