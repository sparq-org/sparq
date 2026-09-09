Correction **6334b338587fe5c635c69a09e134917ec35eaaca** is clean and review-ready: one source file, +18/-13. The default function and tombstone insertion retain exact base compiled text; the opt-in algorithm is unchanged. No declaration has been added to source.

- Native validation: 4 feature-off + 9 feature-on tests pass; all six compiled controls are killed; all-target core clippy passes off/on.
- Original compiler protocol: **declared**, exit 0 in 544.822s; no retry/cutoff. Minimum observed free disk 11,595,014,144 bytes.
- Addition-neutral equals head; deletion-neutral equals base. All four bundles are 1,560,265 bytes. Base/head differ 12 bytes; the two successful obligations establish the repository protocol's metadata-only classification.
- Exact proposal: `proof/declaration-proposal.json`, outside the source tree. Its inherited `[OPUS-5]` template is not execution provenance: Astra ran this proof; actual independent review is pending.

The prior Linux b86 bundles were 24 bytes smaller; no cross-host byte comparison or full CI result is claimed. Default-off experimental costs and remaining full Linux validation are unchanged. Root owns actual Opus review, any declaration/publication, and normal CI.
