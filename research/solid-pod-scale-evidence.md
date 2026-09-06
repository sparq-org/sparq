# Evidence boundaries for the Solid Pod capacity study

[GPT-6] This design record separates observed evidence, source-informed schema
choices and prospective assumptions. The quantitative protocol lives in
[`bench/ac/million/protocol.json`](../bench/ac/million/protocol.json); the
population-to-request calculation lives in
[`workload.json`](../bench/ac/million/workload.json). Derived values are generated
by `bench/ac/million/derive-paper-inputs.py`, with source hashes and locators.

## What the sources establish

| Source | Supported use | Limitation |
| --- | --- | --- |
| [Google Takeout documentation](https://support.google.com/accounts/answer/3024190?hl=en) | Service categories, contacts export format and separate media metadata | An export schema says nothing about per-person retained volume. |
| [Open Banking transaction schema](https://openbankinguk.github.io/read-write-api-site3/v4.0.1/resources-and-data-models/aisp/Transactions.html) | Transaction fields and account relationships | Transaction frequency, retention and coverage of a person's finances require additional evidence. |
| [Federal Reserve payment diary](https://frbservices.org/binaries/content/assets/crsocms/news/research/2026-diary-consumer-payments-choice.pdf) | Observed consumer payment means, used to anchor transaction volume | Rounded U.S. means do not establish retained record counts, the synthetic tail or other countries; the record-per-payment conversion is assumed. |
| [Geolife user guide](https://www.microsoft.com/en-us/research/publication/geolife-gps-trajectory-dataset-user-guide/) | A public example of timestamped location histories | Historical volunteer sampling is not a modern population distribution; choosing location summaries also changes volume. |
| [MovieLens stable dataset](https://grouplens.org/datasets/movielens/1m/) | An empirical marginal distribution of retained rating records | Selected rating-service users, historical collection and minimum participation; cannot stand in for photos, messages or all-service activity. |
| [Dona messaging donations](https://doi.org/10.3758/s13428-024-02593-z) | Retained message counts and spans motivate the messaging-volume sensitivity | Young, selected donors and selected WhatsApp chats do not supply population rates or a joint service-data distribution. |
| [Google Photos engineering](https://cloud.google.com/blog/products/databases/google-photos-builds-user-experience-on-spanner) | Metadata serves interactive services and batch pipelines | Establishes the existence of both work types, not their per-person frequency in Solid. |
| [Andrews et al.](https://doi.org/10.1371/journal.pone.0139004) | Objective interaction counts and substantial between-person variation | Small historical Android cohort; device interactions are not backend requests. The numeric observation and source locator are in the workload file. |
| [Ofcom Online Nation](https://www.ofcom.org.uk/media-use-and-attitudes/online-habits/from-apps-to-ai-search-how-the-uk-goes-online-in-2025) | Breadth and heterogeneity of contemporary service use | Online time and app reach do not identify Pod storage volume or requests per second. |
| [Scaling Memcache at Facebook](https://www.usenix.org/system/files/conference/nsdi13/nsdi13-final170_update.pdf) | Why fanout, batching and caching belong in a demand model | Historical cache traffic cannot be transferred as a Solid workload. |

The main corpus combines an observed rating-count marginal and payment mean with
a deliberately specified multi-service history model. Its other volumes, shared activity factors,
retention and access-control frequencies remain assumptions. Validation reports
must name the supported marginal and the unvalidated dimensions individually.
Neither successful schema validation nor a large generated population warrants
calling the complete corpus a validated representation of future Solid users.

## Access-control justification

The frozen workload models owner-private service records, calendar sharing,
shared media metadata and public ratings. These scenarios motivate owners,
named recipients, fixed recipient groups, inherited permissions and private
exceptions; they do not establish their prevalence. The executed campaign keeps
membership, hierarchy and policy placement fixed. Owner administration changes
calendar grants and checks revocation, eviction and restart. Broader permission
and demand sweeps belong to the prospective protocol, not the frozen results.

Both serializers implement a neutral effective-rights relation. WAC inheritance
stops at the nearest applicable ACL; ACP gathers applicable policies and resolves
allows with denies. A private child of a shared folder can consequently require
different representations. Language syntax is not the comparison unit. A deny
used to obtain the same intended private exception belongs in that paired
scenario; additional context conditions without an equivalent WAC configuration
belong in a separate feature experiment.

The oracle evaluates generated scenario records directly. A second compiler is
not automatically an independent oracle, and agreement between WAC and ACP is
insufficient if both grant the same unauthorized access. Policy checks cover
positive and negative principals, content writes, the shared owner-only policy
administration boundary, and revocation across
cache hits, eviction and restart. If reference queries use the same SPARQL engine,
the supported independence claim concerns authorization selection.

## A capacity result is a conjunction

The storage result requires populated, verified, persistent Pods. The interactive
capacity result additionally requires a justified offered workload, complete
authenticated requests, mixed operations, queueing and failure accounting, and
the declared response target under measured resource limits. A component test can
isolate an implementation cost, but does not satisfy a missing part of that
conjunction. Failure to fit the chosen corpus or sustain its offered demand is a
result, not a reason to reduce corpus volume after observing performance.

Existing canonical observations remain attached to their original compact WAC
study and its original implementation. They motivate the new routing and storage
design and remain a smaller-scale reference. They are not relabeled as measurements
of the new server, ACP support, real networks or the richer personal corpus.

## Rendering the review manuscript

The standalone source is `site/papers/solid-pod-scale.typ`, with its own
bibliography. It does not register another publication or modify the earlier
paper factory's evidence scope. Run `bench/ac/million/derive-paper-inputs.py`
after combining the corpus and paper changes; the generated corpus snapshot
records its input checksum. Then compile with the pinned Typst compiler and the
repository as its root:

```sh
typst compile --root . site/papers/solid-pod-scale.typ output/pdf/solid-pod-scale.pdf
```

The default binding is the finalized [main analysis](solid-pod-scale-main.json),
with a separately hashed [audit summary](solid-pod-scale-main-report.json). Compile
the exhaustive companion beside the manuscript:

```sh
typst compile --root . site/papers/solid-pod-scale-companion.typ output/pdf/solid-pod-scale-companion.pdf
```

The manuscript links to the sibling companion and local source bundle. Its
main response table distinguishes queue-only guard misses from failed delivery;
full matrices, resource counters and indexed-component diagnostics remain in the
companion and JSON. An explicit empty `--input campaign=` selects the older pilot
fallback; partial or unreviewed supplied evidence cannot emit final main claims.
Both PDFs are local review artifacts, not automatic site publications.
