# Evidence ledger: access-controlled SPARQL over many Solid Pods

**Audience:** paper authors and reviewers

**Evidence cut-off:** 2026-09-03
**Scope:** server-side, read-only SPARQL evaluated over a WAC-authorized RDF dataset

This is the claim-to-source ledger for the many-Pod paper. It separates facts established
by external sources, facts established by the pinned SPARQ implementation, and hypotheses
that require the canonical benchmark. A workload value borrowed from another study is a
calibration point, not evidence that it is representative of a future Pod population.

## Consequential-claim gap matrix

| Claim family | Current evidence | Confidence | Contradiction or boundary | Next evidence action |
|---|---|---:|---|---|
| Solid and WAC status/semantics | Published Solid CG reports; WAC v1.0.0 and Solid Protocol v0.11.0, both dated 2024-05-12 | High | Community Group drafts, not W3C Recommendations | Cite exact status; do not call either a W3C Standard |
| Authorized query dataset | Paper's local draft specification plus implementation/conformance tests | High for SPARQ; low for ecosystem adoption | No ratified Solid server-side SPARQL surface | Label the query interface and one-resource/one-graph mapping as this work |
| SPARQL evaluation complexity | Pérez, Arenas, and Gutierrez, ACM TODS 2009 | High | Results concern the formal graph-pattern language, not a concrete engine's runtime | Keep worst-case combined/data complexity separate from implementation cost |
| Access-controlled SPARQL precedent | SAFE (2017); Kirrane et al. (2020) | High | SAFE is federated clinical cubes; Kirrane et al. study query rewriting; neither is WAC over co-hosted Pods | Position contributions by deployment and enforcement boundary, not as the first access-controlled SPARQL system |
| Social Pod counts | SolidBench article and artefact: 1,531 vaults, 158,233 RDF files, 3,556,159 triples | High | Generated from LDBC SNB; not observations of deployed Pods | Call matching settings count-calibrated, not SolidBench-derived or population-representative |
| Compact health counts | TIDAL: 256 participant Pods, each with one RDF/Turtle file containing 128 generated clinical variables/values | Medium-high | The mandatory corpus matches participant/file counts only; its `T=128` means triples, not variables, and is not TIDAL-derived | Call it a participant/file anchor; do not equate variables and triples or generalize to typical patient records |
| Structurally rich health data | Synthea generates synthetic longitudinal EHRs in FHIR/CSV and other formats | High | Not used by the mandatory controlled benchmark; realism is model-dependent | If added, report conversion code and realized distributions; otherwise state as future/supplementary work |
| Independent Solid stress ranges | ESPRESSO varies 0.5--50 MB/Pod, 1--10,000 files/Pod, 10--100% access, and 1--100 Pods | High | Keyword search and a different architecture | Use only to justify sensitivity/stress ranges, not expected production values |
| Measurement methodology | Mytkowicz et al. 2009; Kalibera and Jones 2013; Papadopoulos et al. 2021 | High | No single design removes all cloud variance | Preserve randomized order, hierarchical blocks, raw repetitions, effect-size intervals, and machine metadata |
| Minimal unrelated-Pod overhead | Prospective H2 margin and implementation reasoning | Awaiting data | Conditional on routing to a Pod-sized store and holding the authorized slice fixed | Canonical timing campaign plus cluster bootstrap on one controlled EC2 host |
| Native endpoint server-wide lower bound | Source audit of current LWS root traversal plus deterministic backend counters | High for pinned commit | Characterizes the current baseline, not an inherent lower bound for WAC/SPARQL | Cite exact source revision and corroborate with instrumentation profile |
| Future deployed Pod distribution | No defensible population dataset located | Low/unknown by definition | Count anchors cannot fill this gap | State non-identifiability as a limitation; keep controlled, calibrated, and stress lanes distinct |

## External primary sources

### Standards and interface context

1. **Solid Protocol**, W3C Solid Community Group, Draft Community Group Report,
   version 0.11.0, 2024-05-12. <https://solidproject.org/TR/protocol>
   - Supports the status/version claim and the resource-oriented Solid storage context.
   - Access note: living latest-version URI; the dated version is
     <https://solidproject.org/TR/2024/protocol-20240512>.
2. **Web Access Control**, W3C Solid Community Group, Draft Community Group Report,
   version 1.0.0, 2024-05-12. <https://solidproject.org/TR/wac>
   - Supports grant-otherwise-deny evaluation, `acl:Read`, `acl:accessTo`,
     `acl:default`, public/named-agent subjects, and closest-container inheritance.
   - Access note: exact dated version is
     <https://solidproject.org/TR/2024/wac-20240512>.
3. **Access Control Policy (ACP)**, W3C Solid Community Group, Editor's Draft,
   version 0.9.0, 2022-05-18. <https://solidproject.org/TR/acp>
   - Establishes ACP as a separate draft policy language. It does not justify pooling ACP
     measurements with WAC.
4. Harris, S., and Seaborne, A. **SPARQL 1.1 Query Language**, W3C Recommendation,
   2013-03-21. <https://www.w3.org/TR/sparql11-query/>
   - Supports RDF-dataset, default-graph, named-graph, and query-language semantics.
5. Clark, K. G., Feigenbaum, L., and Torres, E. **SPARQL 1.1 Protocol**, W3C
   Recommendation, 2013-03-21. <https://www.w3.org/TR/sparql11-protocol/>
   - Supports the standardized HTTP query operation used by the draft interface.

### Theory and security criteria

6. Pérez, J., Arenas, M., and Gutierrez, C. **Semantics and Complexity of SPARQL**,
   *ACM Transactions on Database Systems* 34(3), Article 16, 2009.
   <https://doi.org/10.1145/1567274.1567278>
   - General graph-pattern evaluation is PSPACE-complete (Corollary 3.5).
   - For every fixed graph-pattern expression, evaluation is in LOGSPACE in data size
     (Theorem 3.6).
   - UNION-free well-designed patterns are coNP-complete in combined complexity
     (Theorem 4.6). These statements must not be rewritten as concrete time bounds.
7. Kirrane, S., Mileo, A., Polleres, A., and Decker, S. **Query Based Access Control
   for Linked Data**, arXiv:2007.00461, 2020. <https://arxiv.org/abs/2007.00461>
   - Supplies relevant correctness/security criteria for restricting SPARQL by query
     rewriting. The enforcement technique differs from physical authorized-dataset views.
8. Khan, Y., Saleem, M., Mehdi, M., Hogan, A., et al. **SAFE: SPARQL Federation over
   RDF Data Cubes with Access Control**, *Journal of Biomedical Semantics* 8, 5, 2017.
   <https://doi.org/10.1186/s13326-017-0112-6>
   - Establishes graph-level, policy-aware SPARQL federation over clinical/statistical
     RDF data cubes. It is not Solid/WAC and does not study co-hosted-Pod isolation.

### Workload calibration

9. Taelman, R., and Verborgh, R. **Link Traversal Query Processing over Decentralized
   Environments with Structural Assumptions**, in *ISWC 2023*, pp. 3--22.
   <https://doi.org/10.1007/978-3-031-47240-4_1>
   Canonical article: <https://comunica.github.io/Article-ISWC2023-SolidQuery/>
   - Reports SolidBench's default 1,531 vaults, 158,233 RDF files, and 3,556,159
     triples; data and query templates derive from LDBC SNB.
10. Erling, O., Averbuch, A., Larriba-Pey, J., Chafi, H., Gubichev, A., Prat, A.,
    Pham, M.-D., and Boncz, P. **The LDBC Social Network Benchmark: Interactive Workload**,
    *SIGMOD 2015*. <https://doi.org/10.1145/2723372.2742786>
    - Establishes the social-network workload underlying SolidBench; it does not
      establish deployed Solid Pod distributions.
11. Sun, C., Gallofré Ocaña, M., van Soest, J., Dumontier, M., et al. **ciTIzen-centric
    DAta pLatform (TIDAL): Sharing distributed personal data in a privacy-preserving
    manner for health research**, *Semantic Web* 14(3), 2023.
    <https://doi.org/10.3233/SW-223220>
    - Evaluates up to 256 participant Pods and 128 requested variables across three Pod
      providers; supports a participant/file-count calibration point, not a mapping from
      each reported variable to one generated RDF triple.
12. Walonoski, J., Kramer, M., Nichols, J., Quina, A., Moesel, C., Hall, D., Duffett,
    C., Dube, K., Gallagher, T., and McLachlan, S. **Synthea: An approach, method, and
    software mechanism for generating synthetic patients and the synthetic electronic
    health care record**, *JAMIA* 25(3), 230--238, 2018.
    <https://doi.org/10.1093/jamia/ocx079>
    - Supports Synthea as a synthetic longitudinal EHR generator, not a claim that its
      output reproduces any particular future Pod population.
13. Ragab, M., Savateev, Y., Oliver, H., Tiropanis, T., Poulovassilis, A., Chapman, A.,
    Taelman, R., and Roussos, G. **Decentralized Search over Personal Online Datastores:
    Architecture and Performance Evaluation**, in *ICWE 2024*, pp. 49--64.
    <https://doi.org/10.1007/978-3-031-62362-2_4>
    - Reports the independent ESPRESSO stress factors: 0.5--50 MB/Pod, 1--10,000
      files/Pod, 10/25/50/100% accessibility, and 1--100 Pods over 1--50 servers.

### Experimental method

14. Mytkowicz, T., Diwan, A., Hauswirth, M., and Sweeney, P. F. **Producing Wrong Data
    Without Doing Anything Obviously Wrong!**, *ASPLOS 2009*, pp. 265--276.
    <https://doi.org/10.1145/1508244.1508275>
    - Demonstrates measurement bias from innocuous setup changes and motivates complete
      configuration randomization.
15. Kalibera, T., and Jones, R. E. **Rigorous Benchmarking in Reasonable Time**,
    *ISMM 2013*, pp. 63--74. <https://doi.org/10.1145/2464157.2464160>
    - Motivates hierarchical experimental units, deliberate repetition at influential
      levels, and effect-size confidence intervals. The paper's median/block bootstrap is
      a stated adaptation, not an implementation of their exact estimator.
16. Papadopoulos, A. V., Versluis, L., Bauer, A., Herbst, N., von Kistowski, J.,
    Ali-Eldin, A., Abad, C., Amaral, J. N., Tůma, P., and Iosup, A.
    **Methodological Principles for Reproducible Performance Evaluation in Cloud
    Computing**, *IEEE Transactions on Software Engineering* 47(8), 1528--1543, 2021.
    <https://doi.org/10.1109/TSE.2019.2927908>
    - Supports reporting cloud environment/configuration, repetitions, uncertainty, and
      reproducibility controls.

## Repository evidence still to bind at the benchmark commit

- `sparq-solid::PodStore::materialize_wac` and `query_as` for the materialized view.
- `sparq-engine` named-graph iteration behavior for `GRAPH ?g`.
- `sparq-lws-core` `/sparql` handler's server-root containment walk, authorization plan,
  parse/load/evaluate/serialize sequence, and read-lock lifetime.
- Deterministic generator invariants and cross-layer exact-result tests.
- Raw canonical JSONL checksums, environment manifest, cost ledger, analysis command,
  derived CSV/JSON/SVG hashes, and paper evidence pointers.

## Search stopping record

Discovery covered official Solid/WAC/ACP and SPARQL reports, the original SPARQL
complexity paper, access-controlled SPARQL precedents, Solid social/health/stress
benchmarks, a synthetic-EHR generator, and systems/cloud measurement methodology.
Targeted follow-up resolved exact versions, workload denominators, and complexity
qualifiers. Further broad search is unlikely to change the experimental design; the only
material open evidence is the study's own canonical measurement campaign.
