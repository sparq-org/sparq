# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/sparq-org/sparq/releases/tag/v0.1.0) - 2026-09-03

### Added

- resolve target issue #3169 [opus5] ([#4073](https://github.com/sparq-org/sparq/pull/4073))
- *(sparq-substrate)* sparq-substrate/engine: DateTime-kind residual partial order — ind (sq-2k5py) ([#4012](https://github.com/sparq-org/sparq/pull/4012))
- *(substrate)* add Num::cmp_relational + wire sparq-reason (sq-v5evr) [OPUS-4.8] ([#1646](https://github.com/sparq-org/sparq/pull/1646))
- *(substrate)* Kani order-law harnesses for compare_terms — 3 laws proved, mixed-kind intransitivity FOUND + pinned (sq-sqtk2.4) [FABLE-5]/[SONNET-4.6] ([#1477](https://github.com/sparq-org/sparq/pull/1477))
- *(substrate)* make sparq-substrate publishable + join/numeric bench + SKILL (sq-qonbz.4) [SONNET-4.6] ([#1414](https://github.com/sparq-org/sparq/pull/1414))
- *(substrate)* delta-aware persistent build-side JOIN SEAM in sparq-substrate::join (sq-qonbz.1) ([#1347](https://github.com/sparq-org/sparq/pull/1347))
- *(substrate)* scaffold sparq-substrate leaf crate — shared row/key + numeric-tower types (sq-fmprw) [OPUS-4.8] ([#1290](https://github.com/sparq-org/sparq/pull/1290))

### Fixed

- *(numeric)* xsd:float promotion + parse must round to single precision ONCE ([#3796](https://github.com/sparq-org/sparq/pull/3796)) ([#3799](https://github.com/sparq-org/sparq/pull/3799))
- *(engine)* datatype-aware numeric cache + trim-consistent lenient seams — cache ⟺ of_literal on =/</> (sq-74oy4 + sq-6b1lj) [FABLE-5] ([#1822](https://github.com/sparq-org/sparq/pull/1822))
- *(core)* align numeric-value cache parse with the evaluator's XSD acceptance set (sq-9781x) [FABLE-5] ([#1792](https://github.com/sparq-org/sparq/pull/1792))
- *(engine)* kind-first total order for compare_terms — restores transitivity (sq-wjl8i) [FABLE-5] ([#1502](https://github.com/sparq-org/sparq/pull/1502))
- *(engine)* as_num via parse_xsd_f64 (INF/NaN path-agreement) + measured XSD double lexical policy (sq-rkzhr) [OPUS-4.8] ([#1349](https://github.com/sparq-org/sparq/pull/1349))
- *(substrate)* exact_cmp recheck on CompareTerm so ORDER BY/MIN/MAX agree with =/< on f64-collapsed numerics (sq-rikm7) [OPUS-4.8] ([#1325](https://github.com/sparq-org/sparq/pull/1325))
- *(engine)* fn:round float-tier double-rounding (0.49999999999999994 -> 0) (sq-l11x2) [OPUS-4.8] ([#1323](https://github.com/sparq-org/sparq/pull/1323))

### Other

- *(sparq-substrate)* sparq-substrate: single-column join-key equality goes through an outlined ind ([#3679](https://github.com/sparq-org/sparq/pull/3679))
- *(substrate)* cover negative Dec order laws [GPT-5.6]
- *(substrate)* proptest total-order axioms + numeric tower vs exact reference (sq-3dyje.1) ([#1893](https://github.com/sparq-org/sparq/pull/1893))
- *(substrate)* single-column JoinKeys key fast path — cut hash_probe descriptor-projection overhead (sq-4r8uy) ([#1859](https://github.com/sparq-org/sparq/pull/1859))
- *(substrate)* zero-overhead delta harness (substrate.overhead_<kernel>) — the §8 producing bead [FABLE-5] (sq-atjue) ([#1810](https://github.com/sparq-org/sparq/pull/1810))
- *(substrate)* kill 19 surviving mutants in numeric.rs + join.rs (sq-qcnn.40) [SONNET-4.6] ([#1731](https://github.com/sparq-org/sparq/pull/1731))
- *(substrate)* hash-join single hash + batch emission contract (sq-7d3dj.19) ([#1726](https://github.com/sparq-org/sparq/pull/1726))
- *(deps)* Bump criterion from 0.5.1 to 0.8.2 ([#1634](https://github.com/sparq-org/sparq/pull/1634))
- *(substrate)* LFTJ kernel micro-opts — branchless wrap, hoisted order indirection, k=3 fast path (sq-7d3dj.20) ([#1716](https://github.com/sparq-org/sparq/pull/1716))
- *(substrate)* Budget cooperative-cancel unit test for join::delta::probe_emit (sq-qonbz.6) [HAIKU-4.5] ([#1421](https://github.com/sparq-org/sparq/pull/1421))
- *(substrate)* raise sparq-substrate coverage floor 80->96 + kill value-tower/join mutants (sq-qcnn.12) [OPUS-4.8] ([#1379](https://github.com/sparq-org/sparq/pull/1379))
- *(substrate)* move the SPARQL term total order into sparq-substrate behind a generic CompareTerm trait (sq-vezew) [OPUS-4.8] ([#1306](https://github.com/sparq-org/sparq/pull/1306))
- *(substrate)* move the four join kernels into sparq-substrate behind a generic JoinKeys descriptor (sq-hknqs) [OPUS-4.8] ([#1300](https://github.com/sparq-org/sparq/pull/1300))
- *(substrate)* move the id-level numeric value tower into sparq-substrate (sq-ev41x) [OPUS-4.8] ([#1296](https://github.com/sparq-org/sparq/pull/1296))
