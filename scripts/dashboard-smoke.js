#!/usr/bin/env node
// [OPUS-4.8] Node smoke-test for the benchmark dashboard's PURE functions (bead sq-ocuf).
// dashboard.js exports its pure helpers under module.exports when require()d from node, so this
// runs them WITHOUT a browser/DOM. It also loads the committed bench/dashboard/metric-labels.json
// and asserts the readable-label plumbing (labelFor / suiteFor / titleFor / buildSummary).
//
// Usage:  node scripts/dashboard-smoke.js     (exit 0 = pass, 1 = fail)
// This is the script dashboard.js's header comment has long referenced; it was previously a
// dangling reference (no file). Now it exists and is the dashboard's CI smoke gate.
'use strict';

const fs = require('fs');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const DASH = path.join(ROOT, 'bench', 'dashboard', 'dashboard.js');
const LABELS = path.join(ROOT, 'bench', 'dashboard', 'metric-labels.json');

let failures = 0;
function ok(cond, msg) {
  if (cond) { console.log('  ok   ' + msg); }
  else { console.error('  FAIL ' + msg); failures++; }
}
function eq(actual, expected, msg) {
  ok(actual === expected, msg + ' (got ' + JSON.stringify(actual) + ')');
}

// ---- load the label map into the global the module reads (it checks window/global) -----------
const labelsFile = JSON.parse(fs.readFileSync(LABELS, 'utf8'));
global.METRIC_LABELS = labelsFile;

// ---- require dashboard.js (module.exports branch returns the pure fns) -----------------------
const D = require(DASH);
ok(D && typeof D.labelFor === 'function', 'dashboard.js exports labelFor');
ok(typeof D.suiteFor === 'function', 'dashboard.js exports suiteFor');
ok(typeof D.titleFor === 'function', 'dashboard.js exports titleFor');
ok(typeof D.buildSummary === 'function', 'dashboard.js exports buildSummary');

// ---- metric-labels.json shape ----------------------------------------------------------------
ok(labelsFile && typeof labelsFile.labels === 'object', 'metric-labels.json has a labels object');
const labels = labelsFile.labels;
const stemCount = Object.keys(labels).length;
ok(stemCount > 100, 'labels map is populated (' + stemCount + ' stems)');
for (const k of Object.keys(labels)) {
  const r = labels[k];
  if (!r.label || !r.suite || !r.unit) {
    ok(false, 'label record ' + k + ' missing label/suite/unit');
    break;
  }
}

// ---- labelFor / suiteFor: the cryptic stems from the bead brief must become readable ----------
// These assert the label map is wired through (not the fallback) for the exact examples cited.
// [OPUS-4.8] (sq-1wrw) WatDiv metrics now carry the scale-factor token `_sf<SF>` (per-commit SF=1)
// so the per-commit + nightly SF tiers form distinct series for the scaling chart. The label map +
// emitter both use the SF-suffixed stem (watdiv_sf1_S1_count_us), so assert against that name.
eq(D.labelFor('watdiv_sf1_S1_count_us'), 'WatDiv S1 (SF=1) — star query, count', 'watdiv_sf1_S1_count_us label');
eq(D.suiteFor('watdiv_sf1_S1_count_us'), 'WatDiv', 'watdiv suite');
ok(/Students/.test(D.labelFor('lubm_q06_count_us')), 'lubm_q06 mentions Students');
eq(D.suiteFor('lubm_q06_count_us'), 'LUBM (reasoning)', 'lubm suite');
ok(/star join/.test(D.labelFor('op_q02_star3_count_us')), 'op_q02_star3 mentions star join');
eq(D.suiteFor('op_q02_star3_count_us'), 'Operators', 'operator suite');
ok(/materialize/.test(D.labelFor('op_q03_chain_materialize_us')), 'mode appears in materialize label');
eq(D.suiteFor('load_s'), 'Pipeline', 'load_s -> Pipeline');
eq(D.suiteFor('wasm_bundle_bytes'), 'Memory / Size', 'wasm_bundle_bytes -> Memory / Size');

// ---- titleFor: raw stem first (transparency) + dataset/query lines ----------------------------
const t = D.titleFor('watdiv_sf1_S1_count_us');
ok(t.split('\n')[0] === 'watdiv_sf1_S1_count_us', 'titleFor leads with the raw stem');
ok(/dataset:/.test(t) && /query:/.test(t), 'titleFor includes dataset + query');

// ---- graceful fallback: an UNLABELLED metric must still get a label + a suite -----------------
const unknown = 'totally_new_metric_count_us';
ok(typeof D.labelFor(unknown) === 'string' && D.labelFor(unknown).length > 0,
   'unlabelled metric falls back to a humanized label');
ok(typeof D.suiteFor(unknown) === 'string' && D.suiteFor(unknown).length > 0,
   'unlabelled metric falls back to a structural suite');

// ---- buildSummary: groups by suite, in GROUP_ORDER, with readable row labels ------------------
const entries = [{
  commit: { id: 'deadbeefcafe', message: 'test', url: '#' },
  date: Date.now(),
  benches: [
    { name: 'load_s', value: 1.2, unit: 's' },
    { name: 'watdiv_sf1_S1_count_us', value: 30, unit: 'us' },
    { name: 'lubm_q06_count_us', value: 99, unit: 'us' },
    { name: 'op_q01_bgp_count_us', value: 5, unit: 'us' }
  ]
}];
const summary = D.buildSummary(entries);
ok(Array.isArray(summary.groups) && summary.groups.length >= 3, 'buildSummary produced groups');
const groupNames = summary.groups.map(function (g) { return g.group; });
ok(groupNames.indexOf('Pipeline') === 0, 'Pipeline is first group');
ok(groupNames.indexOf('WatDiv') !== -1 && groupNames.indexOf('LUBM (reasoning)') !== -1,
   'WatDiv + LUBM suites present as groups');
// each row carries the readable label + a title (raw stem) for the tooltip.
let sawWatdivRow = false;
summary.groups.forEach(function (g) {
  g.rows.forEach(function (r) {
    if (r.name === 'watdiv_sf1_S1_count_us') {
      sawWatdivRow = true;
      ok(r.label === 'WatDiv S1 (SF=1) — star query, count', 'summary row uses readable label');
      ok(r.title.indexOf('watdiv_sf1_S1_count_us') === 0, 'summary row title carries the raw stem');
    }
  });
});
ok(sawWatdivRow, 'watdiv row present in summary');

// ============================================================================================
// [OPUS-4.8] sq-19z8 — per-metric SEMANTIC badges (LUBM regime + result mode).
// semanticBadges() surfaces the label record's `regime` (extensional vs entailed OWL-RL closure)
// and `mode` (count/materialize/json/…) fields — which previously lived ONLY in the title tooltip —
// as small badge descriptors the DOM renders as muted .pill spans next to the metric label.
// ============================================================================================
ok(typeof D.semanticBadges === 'function', 'dashboard.js exports semanticBadges');

// LUBM entailed query (regime=entailed, mode=count): two badges, regime FIRST then mode.
const entBadges = D.semanticBadges('lubm_q04_count_us');
eq(entBadges.length, 2, 'entailed LUBM query -> two badges (regime + mode)');
eq(entBadges[0].kind, 'badge-entailed', 'first badge is the entailed-regime kind');
eq(entBadges[0].text, 'entailed', 'entailed-regime badge text');
ok(/OWL-RL closure|reasoning/i.test(entBadges[0].title), 'entailed badge title explains the reasoning regime');
eq(entBadges[1].kind, 'badge-mode', 'second badge is the result-mode kind');
eq(entBadges[1].text, 'count', 'mode badge text is the result mode');

// LUBM extensional query (regime=extensional): muted extensional kind, distinct from entailed.
const extBadges = D.semanticBadges('lubm_q01_count_us');
eq(extBadges[0].kind, 'badge-extensional', 'extensional LUBM query -> extensional-regime badge');
eq(extBadges[0].text, 'extensional', 'extensional-regime badge text');
ok(/asserted|no entailment|closure/i.test(extBadges[0].title), 'extensional badge title explains no-closure');

// A metric with a mode but NO regime (e.g. BSBM) -> exactly one mode badge, no regime badge.
const modeOnly = D.semanticBadges('bsbm_query01_count_us');
eq(modeOnly.length, 1, 'mode-only metric -> a single (mode) badge');
eq(modeOnly[0].kind, 'badge-mode', 'mode-only metric badge is the mode kind');

// A metric with NEITHER field (load/store/dict) -> no badges (rows stay clean).
eq(D.semanticBadges('load_s').length, 0, 'load_s has no semantic badges');
// An UNLABELLED metric -> no badges (never sprout a misleading pill on the fallback path).
eq(D.semanticBadges('totally_new_metric_count_us').length, 0, 'unlabelled metric has no semantic badges');

// ============================================================================================
// [OPUS-4.8] sq-xvow — featured well-known suites at the TOP (+ competitor seam for sq-t0c3).
// ============================================================================================
ok(typeof D.featuredSuiteOf === 'function', 'dashboard.js exports featuredSuiteOf');
ok(typeof D.buildFeatured === 'function', 'dashboard.js exports buildFeatured');
ok(typeof D.competitorsFor === 'function', 'dashboard.js exports competitorsFor');

// featuredSuiteOf: recognised public suites are featured; engine micro-suites are NOT.
eq(D.featuredSuiteOf('watdiv_sf1_S1_count_us') && D.featuredSuiteOf('watdiv_sf1_S1_count_us').key, 'WatDiv',
   'watdiv metric is featured under WatDiv');
eq(D.featuredSuiteOf('lubm_q06_count_us') && D.featuredSuiteOf('lubm_q06_count_us').key, 'LUBM',
   'lubm metric is featured under LUBM');
eq(D.featuredSuiteOf('sp2b_q1_count_us') && D.featuredSuiteOf('sp2b_q1_count_us').key, 'SP2Bench',
   'sp2b metric is featured under SP2Bench');
ok(D.featuredSuiteOf('op_q01_bgp_count_us') === null, 'operator micro-suite is NOT featured');
ok(D.featuredSuiteOf('load_s') === null, 'pipeline metric is NOT featured');
// Deep Taxonomy isn't in the label map yet (sq-1hgz) — name-token fallback must still recognise it.
eq(D.featuredSuiteOf('deeptax_d5_count_us') && D.featuredSuiteOf('deeptax_d5_count_us').key, 'Deep Taxonomy',
   'unlabelled deep-taxonomy metric is featured via name fallback');
// [OPUS-4.8] sq-v02y: the 4th capability-surface suite — Vector / ANN recall-deficit metrics are
// featured (via the label-map suite "Vector / ANN" AND the `vectors`/`vector` name-token alias).
eq(D.featuredSuiteOf('vectors_hnsw_recall_at10') && D.featuredSuiteOf('vectors_hnsw_recall_at10').key, 'Vector / ANN',
   'vector ANN recall metric is featured under Vector / ANN');
eq(D.featuredSuiteOf('vectors_diskann_query_us') && D.featuredSuiteOf('vectors_diskann_query_us').key, 'Vector / ANN',
   'vector ANN advisory-latency metric is featured under Vector / ANN');
// [GPT-5.6] sq-g3n7h: every newly implemented comparative suite must resolve from both its
// registry spelling and the underscore-prefixed metric spelling emitted into benchmark feeds.
[
  ['wasm-compare/bundle', 'Browser / WASM'],
  ['wasm_compare_bundle_bytes', 'Browser / WASM'],
  ['canon-bench', 'RDFC-1.0 canonicalization'],
  ['canon_poison_guard_count', 'RDFC-1.0 canonicalization'],
  ['kge-quality-comparison', 'KGE quality'],
  ['kge_filtered_mrr', 'KGE quality'],
  ['gsp-bench', 'Graph Store Protocol'],
  ['gsp_put_p50_ms', 'Graph Store Protocol'],
  ['python-bindings-bench', 'Python bindings'],
  ['python_query_floor_us', 'Python bindings']
].forEach(function (c) {
  const suite = D.featuredSuiteOf(c[0]);
  eq(suite && suite.key, c[1], c[0] + ' is routed to its featured comparative suite');
  ok(suite && typeof suite.note === 'string' && suite.note.length > 0,
     c[1] + ' carries an honesty note');
});

// buildFeatured: groups by suite, latest value per metric, competitor SEAM present.
const featEntries = [{
  commit: { id: 'feedfacefeed', message: 'feat test', url: '#' }, date: Date.now(),
  benches: [
    { name: 'load_s', value: 1.2, unit: 's' },                  // NOT featured
    { name: 'op_q01_bgp_count_us', value: 5, unit: 'us' },      // NOT featured
    { name: 'watdiv_sf1_S1_count_us', value: 30, unit: 'µs' },
    { name: 'lubm_q06_count_us', value: 99, unit: 'µs' },
    { name: 'sp2b_q1_count_us', value: 50, unit: 'µs' },
    { name: 'wasm_compare_bundle_bytes', value: 1, unit: 'bytes' },
    { name: 'canon_poison_guard_count', value: 1, unit: 'count' },
    { name: 'kge_filtered_mrr', value: 1, unit: 'ratio' },
    { name: 'gsp_put_p50_ms', value: 1, unit: 'ms' },
    { name: 'python_query_floor_us', value: 1, unit: 'µs' }
  ]
}];
const feat = D.buildFeatured(featEntries, null);
const featSuites = feat.groups.map(function (g) { return g.suite; });
ok(featSuites.indexOf('WatDiv') !== -1 && featSuites.indexOf('LUBM') !== -1 &&
   featSuites.indexOf('SP2Bench') !== -1, 'buildFeatured surfaces WatDiv/LUBM/SP2Bench');
['Browser / WASM', 'RDFC-1.0 canonicalization', 'KGE quality',
 'Graph Store Protocol', 'Python bindings'].forEach(function (suite) {
  ok(featSuites.indexOf(suite) !== -1, 'buildFeatured surfaces ' + suite);
});
ok(featSuites.indexOf('Operators') === -1 && featSuites.indexOf('Pipeline') === -1,
   'buildFeatured excludes non-featured (engine) suites');
// the SEAM: no competitor file -> empty engine list, every row has a competitors:{} object.
ok(Array.isArray(feat.competitorEngines) && feat.competitorEngines.length === 0,
   'no competitor file -> competitorEngines is []');
let sawFeatRow = false;
feat.groups.forEach(function (g) { g.rows.forEach(function (r) {
  if (r.name === 'watdiv_sf1_S1_count_us') {
    sawFeatRow = true;
    eq(r.value, 30, 'featured row carries the latest value');
    ok(r.label === 'WatDiv S1 (SF=1) — star query, count', 'featured row uses the sq-ocuf readable label');
    ok(r.competitors && typeof r.competitors === 'object', 'featured row exposes a competitors seam');
  }
}); });
ok(sawFeatRow, 'watdiv row present in featured view');

// competitor seam wiring (sq-t0c3): a competitor file is mapped onto rows; matches by raw NAME
// AND by canonical stem (name minus _us). Absent numbers stay absent (-> "—" in the DOM).
const compFile = {
  engines: [{ id: 'qlever', label: 'QLever', version: '1.2.3', env: 'CI' }, { id: 'oxigraph' }],
  values: { 'watdiv_sf1_S1_count_us': { qlever: 12 }, 'lubm_q06_count': { oxigraph: 200 } }
};
const featC = D.buildFeatured(featEntries, compFile);
eq(featC.competitorEngines.length, 2, 'competitor file -> two engine columns');
let watdivComp = null, lubmComp = null;
featC.groups.forEach(function (g) { g.rows.forEach(function (r) {
  if (r.name === 'watdiv_sf1_S1_count_us') watdivComp = r.competitors;
  if (r.name === 'lubm_q06_count_us') lubmComp = r.competitors;
}); });
eq(watdivComp && watdivComp.qlever, 12, 'competitor matched by raw metric name');
eq(lubmComp && lubmComp.oxigraph, 200, 'competitor matched by canonical stem (name minus _us)');
ok(watdivComp && watdivComp.oxigraph === undefined, 'unmatched competitor cell stays absent (renders —)');

// ============================================================================================
// [OPUS-4.8] sq-aas7 — nightly-priority featured tier: pickFeaturedSeries / tierDescriptor and the
// buildFeatured `tier` carry. The featured block PREFERS the EC2/nightly series when present, falls
// back to the per-commit gh-runner series otherwise (graceful when EC2 absent).
// ============================================================================================
ok(typeof D.pickFeaturedSeries === 'function', 'dashboard.js exports pickFeaturedSeries');
ok(typeof D.tierDescriptor === 'function', 'dashboard.js exports tierDescriptor');
eq(D.SERIES, 'sparq engine', 'SERIES constant is the per-commit series name');
eq(D.EC2_SERIES, 'sparq engine (EC2)', 'EC2_SERIES constant matches bench-ec2.yml');

// EC2 present + non-empty -> nightly tier chosen, with the EC2 entries.
const ec2Entry = [{ commit: { id: 'ec2', message: 'n', url: '#' }, date: 1,
  benches: [{ name: 'lubm_q06_count_us', value: 80, unit: 'µs' }] }];
const commitEntry = [{ commit: { id: 'cc', message: 'c', url: '#' }, date: 2,
  benches: [{ name: 'lubm_q06_count_us', value: 99, unit: 'µs' }] }];
const pickedNightly = D.pickFeaturedSeries({ entries: { 'sparq engine': commitEntry, 'sparq engine (EC2)': ec2Entry } });
eq(pickedNightly.tier, 'nightly', 'pickFeaturedSeries prefers EC2/nightly when present');
eq(pickedNightly.seriesName, 'sparq engine (EC2)', 'nightly pick reports the EC2 series name');
ok(pickedNightly.hasEc2 === true, 'nightly pick flags hasEc2');
eq(pickedNightly.entries, ec2Entry, 'nightly pick returns the EC2 entries');

// EC2 absent -> graceful fall-back to per-commit.
const pickedCommit = D.pickFeaturedSeries({ entries: { 'sparq engine': commitEntry } });
eq(pickedCommit.tier, 'per-commit', 'pickFeaturedSeries falls back to per-commit when EC2 absent');
ok(pickedCommit.hasEc2 === false, 'per-commit pick flags hasEc2 false (EC2 absent -> graceful)');
eq(pickedCommit.entries, commitEntry, 'per-commit pick returns the per-commit entries');
// EC2 present but EMPTY -> still per-commit (an empty nightly series must not win).
const pickedEmptyEc2 = D.pickFeaturedSeries({ entries: { 'sparq engine': commitEntry, 'sparq engine (EC2)': [] } });
eq(pickedEmptyEc2.tier, 'per-commit', 'an EMPTY EC2 series does not override per-commit');
// missing data object -> per-commit + empty (no throw).
let pickThrew = false; let pickedNone = null;
try { pickedNone = D.pickFeaturedSeries(undefined); } catch (e) { pickThrew = true; }
ok(!pickThrew, 'pickFeaturedSeries(undefined) does not throw');
ok(pickedNone && pickedNone.tier === 'per-commit' && pickedNone.entries.length === 0,
   'pickFeaturedSeries(undefined) -> per-commit, empty entries');

// tierDescriptor: distinct kinds + non-empty text/title.
const tdN = D.tierDescriptor('nightly'); const tdC = D.tierDescriptor('per-commit');
eq(tdN.kind, 'tier-nightly', 'tierDescriptor(nightly) kind');
eq(tdC.kind, 'tier-per-commit', 'tierDescriptor(per-commit) kind');
ok(tdN.text && tdN.title && tdC.text && tdC.title, 'tier descriptors carry text + title');
// [OPUS-4.8] sq-c0kd: the nightly tier must NOT claim "same box" vs the competitor gathers (nightly
// runs c7g.4xlarge, the gathers ran c7g.xlarge — same c7g family, different size). The title must
// say "instance family" and flag the gap so EC2-nightly-vs-competitor is never read as a direct,
// same-box comparison once the tier goes live.
ok(!/same[ -]box/i.test(tdN.title), 'tierDescriptor(nightly) title does not claim "same box"');
ok(/instance family/i.test(tdN.title), 'tierDescriptor(nightly) title says "instance family"');
ok(/not directly comparable/i.test(tdN.title),
   'tierDescriptor(nightly) title flags it is not directly comparable to the competitor cells');

// buildFeatured carries the chosen tier through to the view (and defaults to per-commit).
const featNightly = D.buildFeatured(ec2Entry, null, pickedNightly);
eq(featNightly.tier, 'nightly', 'buildFeatured carries the nightly tier from opts');
ok(featNightly.hasEc2 === true, 'buildFeatured carries hasEc2');
const featDefault = D.buildFeatured(commitEntry, null);
eq(featDefault.tier, 'per-commit', 'buildFeatured defaults to per-commit when no opts (legacy callers)');

// ============================================================================================
// [OPUS-4.8] sq-viby — scaling comparison: size/depth axis derived from the metric NAME.
// ============================================================================================
ok(typeof D.sizeAxisOf === 'function', 'dashboard.js exports sizeAxisOf');
ok(typeof D.buildScalingFamilies === 'function', 'dashboard.js exports buildScalingFamilies');

// sizeAxisOf: recognise depth / scale-factor tokens; ignore incidental digits (S1, q06, star3).
const dt = D.sizeAxisOf('deeptax_d10_count_us');
ok(dt && dt.axisLabel === 'depth' && dt.axis === 10, 'sizeAxisOf reads deep-taxonomy depth (_d10 -> 10)');
const dt2 = D.sizeAxisOf('deep_taxonomy_depth20_count_us');
ok(dt2 && dt2.axisLabel === 'depth' && dt2.axis === 20, 'sizeAxisOf reads _depth20 -> 20');
const sf = D.sizeAxisOf('watdiv_sf100_C1_count_us');
ok(sf && sf.axisLabel === 'scale factor' && sf.axis === 100, 'sizeAxisOf reads WatDiv SF (_sf100 -> 100)');
const sfk = D.sizeAxisOf('watdiv_sf1k_C1_count_us');
ok(sfk && sfk.axis === 1000, 'sizeAxisOf honours k multiplier (_sf1k -> 1000)');
ok(D.sizeAxisOf('watdiv_S1_count_us') === null, 'sizeAxisOf ignores S1 (not a size token)');
ok(D.sizeAxisOf('lubm_q06_count_us') === null, 'sizeAxisOf ignores q06 (not a size token)');
ok(D.sizeAxisOf('op_q02_star3_count_us') === null, 'sizeAxisOf ignores star3 (not a size token)');
// two sizes of the same query collapse to ONE base -> one family.
eq(D.sizeAxisOf('deeptax_d1_count_us').base, D.sizeAxisOf('deeptax_d10_count_us').base,
   'different sizes of the same query share a family base');

// buildScalingFamilies: groups by base, points sorted ascending by axis; single-point family kept.
const scaleEntries = [{
  commit: { id: 'abc', message: 's', url: '#' }, date: Date.now(),
  benches: [
    { name: 'deeptax_d1_count_us', value: 10, unit: 'µs' },
    { name: 'deeptax_d10_count_us', value: 120, unit: 'µs' },
    { name: 'deeptax_d5_count_us', value: 40, unit: 'µs' },     // out of order on purpose
    { name: 'watdiv_sf100_C1_count_us', value: 800, unit: 'µs' }, // single-point family
    { name: 'lubm_q06_count_us', value: 99, unit: 'µs' }          // NOT size-parametrised
  ]
}];
const fams = D.buildScalingFamilies(scaleEntries);
ok(fams.length === 2, 'buildScalingFamilies found exactly the two size-parametrised families');
const dtFam = fams.filter(function (f) { return f.axisLabel === 'depth'; })[0];
ok(dtFam && dtFam.points.length === 3, 'deep-taxonomy family has 3 depth points');
ok(dtFam.points[0].axis === 1 && dtFam.points[2].axis === 10, 'scaling points sorted ascending by axis');
const sfFam = fams.filter(function (f) { return f.axisLabel === 'scale factor'; })[0];
ok(sfFam && sfFam.points.length === 1, 'single-size family is kept (renders a single marker + note)');
ok(fams.every(function (f) { return !/lubm_q06/.test(f.base); }), 'non-size-parametrised metric excluded');

// [OPUS-4.8] (sq-1wrw) The exact names the ci-bench emitter now produces: the per-commit SF=1 tier
// (watdiv_sf1_C1_count_us) and the EC2/nightly SF=1000 tier (watdiv_sf1000_C1_count_us) must
// collapse to ONE scaling family (same base) with two ascending scale-factor points — this is the
// engine-vs-scale-factor axis the bead unblocks. Before this bead both tiers emitted the SF-less
// stem (watdiv_C1_count_us) and COLLIDED into a single series, so no axis could form.
const wdFams = D.buildScalingFamilies([{
  commit: { id: 'sf', message: 's', url: '#' }, date: Date.now(),
  benches: [
    { name: 'watdiv_sf1000_C1_count_us', value: 9000, unit: 'µs' }, // nightly (out of order on purpose)
    { name: 'watdiv_sf1_C1_count_us', value: 30, unit: 'µs' }       // per-commit
  ]
}]);
ok(wdFams.length === 1, 'watdiv SF tiers collapse to exactly ONE scaling family');
ok(wdFams[0].axisLabel === 'scale factor', 'watdiv scaling axis is scale factor');
ok(wdFams[0].points.length === 2, 'watdiv family has the two SF points (per-commit + nightly)');
ok(wdFams[0].points[0].axis === 1 && wdFams[0].points[1].axis === 1000,
   'watdiv SF points sorted ascending (SF=1 -> SF=1000)');
eq(D.sizeAxisOf('watdiv_sf1_C1_count_us').base, D.sizeAxisOf('watdiv_sf1000_C1_count_us').base,
   'watdiv per-commit + nightly share one family base (sf-token stripped)');

// ============================================================================================
// [OPUS-4.8] Copilot review fixes on PR #59 — unit-less labels (#2/#4), clean scaling base (#3),
// and the featured "Metric" column header (#1).
// ============================================================================================
ok(typeof D.labelForBare === 'function', 'dashboard.js exports labelForBare');

// #2/#4 — labelForBare strips ONLY a trailing ` (<unit>)` so the unit isn't doubled when the unit
// has its OWN cell/column. An UNLABELLED metric (humanize fallback) appends `(µs)`/`(s)` to its
// label; labelForBare must remove it. Labelled metrics (sq-ocuf) carry no trailing unit -> unchanged.
ok(/\(µs\)$/.test(D.labelFor('deeptax_d10_count_us')), 'labelFor (fallback) carries a trailing (µs)');
ok(!/\(µs\)$/.test(D.labelForBare('deeptax_d10_count_us')),
   'labelForBare strips the trailing (µs) from an unlabelled metric');
ok(!/\(s\)$/.test(D.labelForBare('totally_new_metric_s')),
   'labelForBare strips a trailing (s) too');
eq(D.labelForBare('watdiv_sf1_S1_count_us'), D.labelFor('watdiv_sf1_S1_count_us'),
   'labelForBare leaves a labelled (no-trailing-unit) label unchanged');
// mid-label parentheses must survive: only a TRAILING parenthesised token is a unit. lubm_q06's
// readable label mentions "Students" and carries no trailing unit, so it is returned untouched.
eq(D.labelForBare('lubm_q06_count_us'), D.labelFor('lubm_q06_count_us'),
   'labelForBare leaves a labelled metric (no trailing unit) untouched');

// #2 — buildFeatured rows use the unit-less label (Deep Taxonomy is unlabelled -> would double).
const dtFeatEntries = [{
  commit: { id: 'd', message: 'd', url: '#' }, date: Date.now(),
  benches: [{ name: 'deeptax_d10_count_us', value: 120, unit: 'µs' }]
}];
const dtFeat = D.buildFeatured(dtFeatEntries, null);
let dtFeatRow = null;
dtFeat.groups.forEach(function (g) { g.rows.forEach(function (r) {
  if (r.name === 'deeptax_d10_count_us') dtFeatRow = r;
}); });
ok(dtFeatRow, 'featured view surfaces the deep-taxonomy row');
ok(dtFeatRow && !/\(µs\)$/.test(dtFeatRow.label),
   'featured row label has NO trailing unit (unit lives in its own column) — fix #2');

// #4 — scaling family label is unit-less (renderScaling appends the unit to the card title).
const dtScaleFam = D.buildScalingFamilies(scaleEntries).filter(function (f) {
  return f.axisLabel === 'depth';
})[0];
ok(dtScaleFam && !/\(µs\)$/.test(dtScaleFam.label),
   'scaling family label has NO trailing unit (title appends it) — fix #4');

// #3 — sizeAxisOf base is normalized: no double/leading/trailing underscores, even though removing
// the token leaves a placeholder `_` adjacent to existing underscores.
eq(D.sizeAxisOf('deeptax_d10_count_us').base, 'deeptax_count_us',
   'sizeAxisOf base collapses the placeholder underscore (no `deeptax__count_us`) — fix #3');
ok(!/__/.test(D.sizeAxisOf('watdiv_sf100_C1_count_us').base), 'no double underscore in scaling base');
ok(!/^_|_$/.test(D.sizeAxisOf('deeptax_d1_count_us').base), 'no leading/trailing underscore in base');
// the base is still a SHARED key across sizes (fix #3 must not break family grouping).
eq(D.sizeAxisOf('deeptax_d1_count_us').base, D.sizeAxisOf('deeptax_d10_count_us').base,
   'normalized base is still shared across sizes (family grouping intact)');

// ============================================================================================
// [OPUS-4.8] sq-rltn — capability FAMILIES: the summary-first taxonomy that makes the dashboard
// visibly cover ALL query types (not SPARQL-only). familyOf() routes a metric to a top-level
// family; buildFamilies() builds the per-family view (latest value per metric) for first paint.
// ============================================================================================
ok(Array.isArray(D.FAMILIES) && D.FAMILIES.length >= 7, 'dashboard.js exports FAMILIES taxonomy');
ok(typeof D.familyOf === 'function', 'dashboard.js exports familyOf');
ok(typeof D.buildFamilies === 'function', 'dashboard.js exports buildFamilies');

// every required capability family is present in the taxonomy, in coverage order.
['core', 'sparql', 'shacl', 'geo', 'fts', 'vector', 'reasoning'].forEach(function (k) {
  ok(D.FAMILIES.some(function (f) { return f.key === k; }), 'FAMILIES includes the ' + k + ' family');
});

// [OPUS-4.8] sq-ncvq.15 / drift-scan §5.D: the newly-promoted families that close the dashboard
// coverage gap — the ZK estate (headline) plus Solid/HDT/RSP/GenAI/GPU. They have NO flowing data
// yet (stdout/criterion benches), so they render as "not yet reported"; assert they EXIST in the
// taxonomy AND that a representative (still unlabelled) metric routes to each via the prefix
// fallback, so when a ci-bench hook starts emitting, the metric lands in the right family.
['zk', 'solid', 'hdt', 'rsp', 'genai', 'gpu'].forEach(function (k) {
  ok(D.FAMILIES.some(function (f) { return f.key === k; }), 'FAMILIES includes the new ' + k + ' family (coverage gap closed)');
});
eq(D.familyOf('zk_commit_us').key, 'zk', 'unlabelled zk-commit metric -> ZK family (headline gap)');
eq(D.familyOf('zktrace_bgp_us').key, 'zk', 'unlabelled zk-trace metric -> ZK family');
eq(D.familyOf('zkcompose_gates').key, 'zk', 'unlabelled zk-compose gate-count metric -> ZK family');
eq(D.familyOf('solid_wac_materialize_us').key, 'solid', 'unlabelled solid/WAC metric -> Solid family');
eq(D.familyOf('hdt_load_s').key, 'hdt', 'unlabelled HDT-load metric -> HDT family');
eq(D.familyOf('rsp_throughput').key, 'rsp', 'unlabelled RSP-throughput metric -> RSP family');
eq(D.familyOf('sim_most_similar_us').key, 'genai', 'unlabelled similarity metric -> GenAI family');
eq(D.familyOf('introspect_build_s').key, 'genai', 'unlabelled introspection metric -> GenAI family');
eq(D.familyOf('gpu_filter_us').key, 'gpu', 'unlabelled GPU-kernel metric -> GPU family');
// [OPUS-4.8] sq-5o5.5-.9: the featured=false trend-only dispositions. NL→SPARQL (nlq) is part of the
// GenAI estate; MPC gets its OWN caveated trend family (modelled counting tier, semi-honest). The
// sim/introspect/nlq/mpc benches are flagged `featured = false` in bench/benchmarks.toml (trend-only,
// NO competitor card), and route to a real dashboard family so coverage is HONEST, not silent.
ok(D.FAMILIES.some(function (f) { return f.key === 'mpc'; }), 'FAMILIES includes the new mpc family (trend-only, featured=false)');
eq(D.familyOf('nlq_ground_us').key, 'genai', 'unlabelled NL→SPARQL (nlq) metric -> GenAI family');
eq(D.familyOf('nlq_ask_us').key, 'genai', 'unlabelled nlq ask-loop metric -> GenAI family');
eq(D.familyOf('mpc_rounds').key, 'mpc', 'unlabelled modelled-MPC rounds metric -> MPC family');
// the load-bearing one: an MPC `_bytes` counting-tier metric must NOT be sunk into Core by the
// generic "Memory / Size" STRUCTURAL fallback — its `mpc` capability prefix wins (sq-5o5.8).
eq(D.familyOf('mpc_join_bytes').key, 'mpc', 'modelled-MPC byte-count metric -> MPC family (capability prefix beats the structural Memory/Size fallback)');
eq(D.familyOf('mpcmatrix_total_bytes').key, 'mpc', 'mpc-matrix byte-count metric -> MPC family');
// wasm-bundle is a deterministic SIZE GATE, not its own capability card: wasm_bundle_bytes stays in
// Core (the `wasm` token is a Core prefix + its Memory/Size suite) — featured=false only means "no
// competitor card", it does not move the size gate out of Core (sq-5o5.9).
eq(D.familyOf('wasm_bundle_bytes').key, 'core', 'wasm bundle-size byte gate -> Core family (size gate, NOT a separate capability card)');
// selective + u64 are SPARQL micro-suites (cli `bench` q01_* stems) — recognise them under SPARQL.
eq(D.familyOf('selective_q01_count_us').key, 'sparql', 'unlabelled selective-join metric -> SPARQL family');
eq(D.familyOf('u64_q03_materialize_us').key, 'sparql', 'unlabelled u64 value-id metric -> SPARQL family');
// the existing capability routings must be UNCHANGED by the new families (no mis-bucketing).
eq(D.familyOf('rdfs_infer_s').key, 'core', 'rdfs_infer_s still routes to Core (Pipeline suite wins)');

// familyOf: each query type lands in its OWN family (SPARQL volume must not swallow capabilities).
eq(D.familyOf('watdiv_sf1_S1_count_us').key, 'sparql', 'watdiv -> SPARQL family');
eq(D.familyOf('sp2b_q1_count_us').key, 'sparql', 'sp2b -> SPARQL family');
eq(D.familyOf('lubm_q06_count_us').key, 'sparql', 'lubm -> SPARQL family');
eq(D.familyOf('op_q01_bgp_count_us').key, 'sparql', 'operators -> SPARQL family');
eq(D.familyOf('shacl_cardinality_validate_us').key, 'shacl', 'shacl -> SHACL family');
eq(D.familyOf('geo_within10km_us').key, 'geo', 'geo -> GeoSPARQL family');
eq(D.familyOf('geo_compliance_deficit').key, 'geo', 'geo non-_us metric -> GeoSPARQL family');
eq(D.familyOf('text_phrase_us').key, 'fts', 'text -> Full-Text family');
eq(D.familyOf('fts_bytes_per_doc').key, 'fts', 'fts non-_us metric -> Full-Text family');
eq(D.familyOf('deeptax_d10_count_us').key, 'reasoning', 'deeptax -> Reasoning family');
eq(D.familyOf('load_s').key, 'core', 'load_s -> Core family');
eq(D.familyOf('store_triples_bytes').key, 'core', 'store bytes -> Core family');
// a not-yet-labelled vector metric routes to the Vector/ANN family via the prefix fallback.
eq(D.familyOf('vectors_recall_at10_us').key, 'vector', 'unlabelled vector metric -> Vector/ANN family');
eq(D.familyOf('ann_query_us').key, 'vector', 'unlabelled ann metric -> Vector/ANN family');

// buildFamilies: one entry per family in order; families with no data are KEPT (count 0) so the
// nav + "not yet reported" section keep coverage honest.
const famEntries = [{
  commit: { id: 'fam', message: 'fam', url: '#' }, date: Date.now(),
  benches: [
    { name: 'load_s', value: 1.2, unit: 's' },
    { name: 'watdiv_sf1_S1_count_us', value: 30, unit: 'µs' },
    { name: 'shacl_cardinality_validate_us', value: 40, unit: 'µs' },
    { name: 'geo_within10km_us', value: 55, unit: 'µs' },
    { name: 'text_phrase_us', value: 22, unit: 'µs' }
    // NB: no vector, no reasoning metric -> those families render with count 0.
  ]
}];
const famView = D.buildFamilies(famEntries);
eq(famView.length, D.FAMILIES.length, 'buildFamilies returns one entry per family');
eq(famView.map(function (f) { return f.key; }).join(','),
   D.FAMILIES.map(function (f) { return f.key; }).join(','),
   'buildFamilies preserves FAMILIES coverage order');
const famByKey = {}; famView.forEach(function (f) { famByKey[f.key] = f; });
ok(famByKey.sparql.count >= 1, 'SPARQL family has rows');
ok(famByKey.shacl.count === 1, 'SHACL family has its one metric');
ok(famByKey.geo.count === 1 && famByKey.fts.count === 1, 'Geo + Full-Text families populated');
ok(famByKey.vector.count === 0, 'Vector/ANN family has count 0 (kept for honest coverage)');
ok(famByKey.reasoning.count === 0, 'Reasoning family has count 0 in this fixture (kept)');
// each row carries the readable label + latest value (the summary first-paint payload).
let sawFamWatdiv = false;
famByKey.sparql.rows.forEach(function (r) {
  if (r.name === 'watdiv_sf1_S1_count_us') {
    sawFamWatdiv = true;
    eq(r.value, 30, 'family row carries the latest value');
    ok(r.label === 'WatDiv S1 (SF=1) — star query, count', 'family row uses the readable label');
  }
});
ok(sawFamWatdiv, 'watdiv row present in the SPARQL family view');

// [OPUS-4.8] Copilot #200: buildFamilies must NOT throw on an empty/absent entries list — it does
// entries[entries.length - 1], which would blow up on []. The docstring promises empty families
// are kept, so an empty input returns one zero-count entry per family (the same shape).
let emptyFamView = null;
let emptyThrew = false;
try { emptyFamView = D.buildFamilies([]); } catch (e) { emptyThrew = true; }
ok(!emptyThrew, 'buildFamilies([]) does not throw on an empty entries list (Copilot #200)');
ok(Array.isArray(emptyFamView) && emptyFamView.length === D.FAMILIES.length,
   'buildFamilies([]) still returns one entry per family');
ok(emptyFamView && emptyFamView.every(function (f) { return f.count === 0 && f.rows.length === 0; }),
   'buildFamilies([]) returns every family with count 0 / no rows');
// also tolerate undefined / no entries at all (defensive, same guard).
let undefThrew = false;
try { D.buildFamilies(undefined); } catch (e) { undefThrew = true; }
ok(!undefThrew, 'buildFamilies(undefined) does not throw either');

// ============================================================================================
// [OPUS-4.8] BROWSER-DOM SIMULATION — confirm the featured section + scaling charts actually
// build DOM (sq-xvow #featured / sq-viby #scaling), like sq-ocuf's renderSummary smoke would. A
// tiny stub document/window/Chart lets the (browser-only) render fns run under node: we import
// dashboard.js a SECOND time with its module.exports branch suppressed so its DOM half executes.
// ============================================================================================
(function domSimulation() {
  // --- minimal DOM shim: just enough for el()/renderFeatured()/renderSummary()/render(). -------
  // [OPUS-4.8] Copilot #200: the shim now models enough of the DOM that the state-preservation
  // logic (querySelector/querySelectorAll over .family-section / details.family-trends[open],
  // parentNode, id, and a real toggle event on `open`) actually exercises in node — so the
  // relabel-preserves-open-state assertion below is a true behavioural check, not a no-op skip.
  function classList(node) { return (node.attributes['class'] || '').split(/\s+/).filter(Boolean); }
  // Tiny CSS-ish matcher supporting the exact selectors dashboard.js uses:
  //   "details.family-trends", "details.family-trends[open]",
  //   "section.family-section[id] > details.family-trends[open]"
  function matchSimple(node, sel) {
    sel = sel.trim();
    if (sel === '*') return true;
    var m = sel.match(/^([a-z]+)?((?:\.[a-z0-9-]+)*)((?:\[[a-z]+\])*)$/i);
    if (!m) return false;
    if (m[1] && node.tagName !== m[1]) return false;
    var cls = (m[2].match(/\.[a-z0-9-]+/gi) || []).map(function (c) { return c.slice(1); });
    var have = classList(node);
    for (var i = 0; i < cls.length; i++) if (have.indexOf(cls[i]) === -1) return false;
    var attrs = m[3].match(/\[([a-z]+)\]/gi) || [];
    for (var j = 0; j < attrs.length; j++) {
      var name = attrs[j].slice(1, -1);
      if (name === 'open') { if (!node.open) return false; }
      else if (!(name in node.attributes) && node[name] == null) return false;
    }
    return true;
  }
  function descendants(node, out) {
    node.children.forEach(function (c) { out.push(c); descendants(c, out); });
    return out;
  }
  function makeNode(tag) {
    return {
      tagName: tag, children: [], attributes: {}, _text: '', _html: '',
      style: {}, _open: false, parentNode: null, _listeners: {},
      get id() { return this.attributes.id; },
      set textContent(v) { this._text = String(v); }, get textContent() { return this._text; },
      set innerHTML(v) { this._html = String(v); if (v === '') this.children = []; },
      get innerHTML() { return this._html; },
      get open() { return this._open; },
      set open(v) {
        var was = this._open; this._open = !!v;
        if (was !== this._open) (this._listeners.toggle || []).forEach(function (fn) { fn({ type: 'toggle' }); });
      },
      get lastChild() { return this.children[this.children.length - 1]; },
      get firstChild() { return this.children[0]; },
      setAttribute: function (k, v) { this.attributes[k] = String(v); },
      appendChild: function (c) { c.parentNode = this; this.children.push(c); return c; },
      addEventListener: function (ev, fn) { (this._listeners[ev] = this._listeners[ev] || []).push(fn); },
      dispatch: function (ev) { (this._listeners[ev] || []).forEach(function (fn) { fn({ type: ev }); }); },
      querySelector: function (sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll: function (sel) {
        var self = this;
        if (sel.indexOf('>') !== -1) {
          var parts = sel.split('>').map(function (s) { return s.trim(); });
          var parent = parts[0], child = parts[1];
          var res = [];
          descendants(self, []).forEach(function (n) {
            if (matchSimple(n, parent)) n.children.forEach(function (c) { if (matchSimple(c, child)) res.push(c); });
          });
          return res;
        }
        return descendants(self, []).filter(function (n) { return matchSimple(n, sel); });
      },
      getContext: function () { return {}; }
    };
  }
  const hosts = {};
  ['updated', 'repo', 'summary', 'charts', 'featured', 'same-box', 'scaling', 'families'].forEach(function (id) {
    hosts[id] = makeNode('div');
  });
  global.document = {
    readyState: 'complete',
    createElement: makeNode,
    getElementById: function (id) {
      if (hosts[id]) return hosts[id];
      // [OPUS-4.8] also resolve dynamically-created ids (e.g. the fam-<key> sections appended into
      // #families) so the relabel state-preservation path can re-find a section by id.
      for (var k in hosts) {
        var found = hosts[k].querySelectorAll('*').filter(function (n) { return n.attributes.id === id; })[0];
        if (found) return found;
      }
      return makeNode('div');
    },
    addEventListener: function () {}
  };
  global.window = {
    matchMedia: function () { return { matches: false }; },
    __SPARQ_DASHBOARD_TEST__: true,   // [OPUS-4.8] expose paintSummary so we can drive a real relabel
    BENCHMARK_DATA: {
      lastUpdate: Date.now(), repoUrl: 'https://github.com/sparq-org/sparq',
      entries: { 'sparq engine': [{
        commit: { id: 'deadbeefcafe', message: 'dom sim', url: '#' }, date: Date.now(),
        benches: [
          { name: 'load_s', value: 1.2, unit: 's' },                        // Core family
          { name: 'watdiv_sf1_S1_count_us', value: 30, unit: 'µs' },        // SPARQL family
          { name: 'lubm_q06_count_us', value: 99, unit: 'µs' },             // SPARQL family
          { name: 'shacl_cardinality_validate_us', value: 40, unit: 'µs' }, // SHACL family
          { name: 'geo_within10km_us', value: 55, unit: 'µs' },             // GeoSPARQL family
          { name: 'text_phrase_us', value: 22, unit: 'µs' },                // Full-Text family
          // size-parametrised family (sq-viby): two depths -> a real scaling curve. (Reasoning)
          { name: 'deeptax_d1_count_us', value: 10, unit: 'µs' },
          { name: 'deeptax_d10_count_us', value: 120, unit: 'µs' }
        ]
      }] }
    },
    METRIC_LABELS: labelsFile
  };
  // a no-op Chart constructor so renderCharts doesn't blow up.
  global.Chart = function () { return {}; };
  // a no-op IntersectionObserver so the lazy-chart path (sq-rltn) runs under node.
  global.IntersectionObserver = function () {
    return { observe: function () {}, unobserve: function () {}, disconnect: function () {} };
  };

  // re-import dashboard.js with its module.exports branch suppressed so the DOM half runs. The
  // dashboard's IIFE takes the node-export early-return when `module.exports` is truthy; inside this
  // eval scope we shadow `module` to undefined so it falls through to the BROWSER DOM path instead.
  // boot() then runs synchronously: there is no window.fetch in the shim, so it calls render().
  const dashSrc = fs.readFileSync(DASH, 'utf8');
  // eslint-disable-next-line no-eval
  (function (module, exports, require) { eval(dashSrc); })(undefined, undefined, undefined);

  ok(hosts.featured.children.length > 0, 'DOM: #featured populated (sq-xvow rendered)');
  ok(hosts.scaling.children.length > 0, 'DOM: #scaling populated (sq-viby rendered)');
  // [OPUS-4.8] sq-ays7: the same-box SPARQL comparison card renders from the embedded
  // COMPETITORS_DATA.same_box_comparisons (the shim has no fetch, so boot() uses the mirror).
  // It renders a section when the mirror carries a comparison, else a graceful hint.
  ok(hosts['same-box'].children.length > 0, 'DOM: #same-box populated (sq-ays7 rendered)');

  // ---- sq-rltn: summary-first FAMILIES render — first paint covers ALL query types -------------
  ok(hosts.families.children.length > 0, 'DOM: #families populated (sq-rltn summary-first render)');
  // a family NAV chip row + one section per family, in FAMILIES order.
  const famNav = hosts.families.children.filter(function (c) { return c.tagName === 'nav'; })[0];
  ok(famNav && famNav.children.length === D.FAMILIES.length,
     'DOM: family nav has a chip per family (' + D.FAMILIES.length + ')');
  const famSections = hosts.families.children.filter(function (c) {
    return c.tagName === 'section' && /^fam-/.test(c.attributes.id || '');
  });
  ok(famSections.length === D.FAMILIES.length,
     'DOM: one section per family rendered (got ' + famSections.length + ')');
  // each REQUIRED capability family must be present as its own section (covers all query types).
  ['core', 'sparql', 'shacl', 'geo', 'fts', 'vector', 'reasoning'].forEach(function (key) {
    ok(famSections.some(function (s) { return s.attributes.id === 'fam-' + key; }),
       'DOM: family section #fam-' + key + ' present');
  });
  // the SPARQL family section holds a summary TABLE at first paint (no chart instantiated yet).
  const sparqlSec = famSections.filter(function (s) { return s.attributes.id === 'fam-sparql'; })[0];
  const sparqlTable = sparqlSec && sparqlSec.children.filter(function (c) { return c.tagName === 'table'; })[0];
  ok(sparqlTable, 'DOM: SPARQL family renders a summary table synchronously (first paint)');
  // [OPUS-4.8] sq-19z8: the LUBM row (lubm_q06_count_us — entailed OWL-RL closure, mode=count)
  // carries its semantic badges as .metric-badge pills next to the label, not just in the tooltip.
  const sparqlBadges = sparqlSec ? sparqlSec.querySelectorAll('span.metric-badge') : [];
  ok(sparqlBadges.length > 0, 'DOM: SPARQL family rows render .metric-badge pills (sq-19z8)');
  ok(sparqlBadges.some(function (b) { return classList(b).indexOf('badge-entailed') !== -1; }),
     'DOM: the entailed LUBM row shows a .badge-entailed regime pill');
  ok(sparqlBadges.some(function (b) { return classList(b).indexOf('badge-mode') !== -1 && b.textContent === 'count'; }),
     'DOM: the LUBM row shows a .badge-mode "count" result-mode pill');
  // every semantic badge reuses the .pill base class (sq-19z8 reuses the Δ-pill styling).
  ok(sparqlBadges.every(function (b) { return classList(b).indexOf('pill') !== -1; }),
     'DOM: semantic badges reuse the .pill base class');
  // first paint must NOT have built any chart: the trend grid lives inside a <details> and is empty
  // until expanded. Confirm the details/summary disclosure exists with an EMPTY chart-grid.
  const sparqlDetails = sparqlSec && sparqlSec.children.filter(function (c) { return c.tagName === 'details'; })[0];
  ok(sparqlDetails, 'DOM: SPARQL family has a lazy <details> Trends disclosure');
  const sparqlGrid = sparqlDetails && sparqlDetails.children.filter(function (c) {
    return (c.attributes['class'] || '') === 'chart-grid';
  })[0];
  ok(sparqlGrid && sparqlGrid.children.length === 0,
     'DOM: trend chart grid is EMPTY at first paint — charts are lazy (not built upfront)');
  // a family with NO data (vector — none in this fixture) renders a "not yet reported" section.
  const vecSec = famSections.filter(function (s) { return s.attributes.id === 'fam-vector'; })[0];
  ok(vecSec && vecSec.children.some(function (c) {
    return (c.attributes['class'] || '').indexOf('family-empty') !== -1;
  }), 'DOM: an empty family (Vector/ANN) renders a "not yet reported" placeholder (honest coverage)');
  // [OPUS-4.8] sq-aas7: the featured host opens with a TIER badge row (the shim has no fetch, so the
  // EC2 series is absent -> the per-commit tier badge), then the suite sections.
  const tierRow = hosts.featured.children.filter(function (c) {
    return (c.attributes['class'] || '').indexOf('featured-tier') === 0;
  })[0];
  ok(tierRow, 'DOM: featured host renders a tier badge row (sq-aas7)');
  const tierPill = tierRow && tierRow.querySelectorAll('span.tier-badge')[0];
  ok(tierPill && classList(tierPill).indexOf('tier-per-commit') !== -1,
     'DOM: tier badge is per-commit (EC2 series absent in the shim -> graceful fall-back)');
  ok(tierPill && classList(tierPill).indexOf('pill') !== -1, 'DOM: tier badge reuses the .pill base class');
  // featured host should contain at least one suite section with a table (after the tier row).
  const featuredSection = hosts.featured.children.filter(function (c) { return c.tagName === 'section'; })[0];
  ok(featuredSection && featuredSection.tagName === 'section', 'DOM: featured host holds suite sections');
  // #1 — the featured table's first column header is "Metric" (not "Query"): the cell may hold
  // non-query metrics. Walk section -> table -> thead -> tr -> first th.
  (function () {
    var table = featuredSection.children.filter(function (c) { return c.tagName === 'table'; })[0];
    var thead = table && table.children.filter(function (c) { return c.tagName === 'thead'; })[0];
    var headRow = thead && thead.children[0];
    var firstTh = headRow && headRow.children[0];
    ok(firstTh && firstTh.textContent === 'Metric',
       'DOM: featured table first column header is "Metric" (fix #1, got ' +
       JSON.stringify(firstTh && firstTh.textContent) + ')');
  })();
  // scaling host should contain a chart-grid with at least one scaling card.
  ok(hosts.scaling.children[0] && hosts.scaling.children[0].attributes['class'] === 'chart-grid',
     'DOM: scaling host holds a chart-grid');

  // ---- Copilot #200: a RELABEL repaint must PRESERVE expanded <details> + re-mount their charts --
  // The user expands the SPARQL family's Trends disclosure; then metric-labels.json resolves and
  // paintSummary() re-runs (the relabel repaint). Before the fix, host.innerHTML='' wiped the open
  // state. After the fix, paintSummary captures open sections and restores them.
  (function relabelPreservesState() {
    const hook = global.window.__sparqDashboardTest;
    ok(hook && typeof hook.paintSummary === 'function', 'DOM: test hook exposes paintSummary for relabel test');
    if (!hook) return;
    // open the SPARQL family <details> (simulate the user expanding it). Setting .open fires toggle,
    // which lazily builds that section's chart grid.
    const sparqlSection = hosts.families.querySelectorAll('section.family-section')
      .filter(function (s) { return s.attributes.id === 'fam-sparql'; })[0];
    const det = sparqlSection && sparqlSection.querySelector('details.family-trends');
    ok(det, 'DOM: found SPARQL family <details> to expand');
    det.open = true;
    const gridBefore = det.querySelector('div.chart-grid');
    ok(gridBefore && gridBefore.children.length > 0, 'DOM: expanding mounts the trend chart cards (lazy build ran)');
    // now drive the REAL relabel repaint (paintSummary re-run).
    hook.paintSummary();
    // the SPARQL section is rebuilt; its <details> must be OPEN again (state preserved).
    const sparqlAfter = hosts.families.querySelectorAll('section.family-section')
      .filter(function (s) { return s.attributes.id === 'fam-sparql'; })[0];
    const detAfter = sparqlAfter && sparqlAfter.querySelector('details.family-trends');
    ok(detAfter && detAfter.open === true,
       'DOM: relabel repaint PRESERVES the expanded SPARQL <details> (Copilot #200 state-preservation)');
    // a family the user did NOT open stays closed (we only restore what was open).
    const coreAfter = hosts.families.querySelectorAll('section.family-section')
      .filter(function (s) { return s.attributes.id === 'fam-core'; })[0];
    const coreDet = coreAfter && coreAfter.querySelector('details.family-trends');
    ok(coreDet && coreDet.open === false, 'DOM: an un-opened family stays closed after repaint (no over-restore)');
    // re-opened section re-mounted its charts (toggle fired on restore).
    const gridAfter = detAfter && detAfter.querySelector('div.chart-grid');
    ok(gridAfter && gridAfter.children.length > 0, 'DOM: restored <details> re-mounts its trend charts');
  })();

  delete global.document; delete global.window; delete global.Chart; delete global.IntersectionObserver;
})();

if (failures) {
  console.error('\nFAILED: ' + failures + ' assertion(s).');
  process.exit(1);
}
console.log('\nAll dashboard smoke checks passed.');
