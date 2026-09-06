<!-- [OPUS-4.8] sq-puyy: trimmed to the concise internal-stub README template (sq-9jw5). -->
# sparq-zk-compose

ZK proof **composition** for [sparq](../../README.md) — stage 2 of the query-proof
design ([`research/zkp-query-proofs-plan.md`](../../research/zkp-query-proofs-plan.md)
v3, §S4.E). Drives the per-property Noir circuit family at
[`zk/compose/`](../../zk/compose) into a full query-result proof
(`manifest::ProofManifest` + the nargo/bb subprocess prover) and verifies one
(`verifier::verify_manifest`). `capture` packages native per-circuit `driver::ProofArtifacts` into the browser-shippable *captured* manifest the `/showcase/zk-car-hire` in-tab verify fallback consumes (sq-1s2.3 / FL1 follow-up) — research-grade, NOT externally audited (sq-qhy4).

> **Internal crate — not published** to crates.io (`publish = false`): nothing in
> the workspace depends on it, so default and wasm builds are byte-identical either way.

<!-- separate, distinct blockquotes: the internal-crate note above vs. the soundness caveat below (MD028) -->

> **NOT-yet-sound** (standing caveat — sq-qhy4 / sq-9hrn; remediation epic sq-1s2).
> No soundness, zero-knowledge, or privacy property is claimed as achieved; the
> verifier's soundness is the subject of the open external audit
> ([external](../../research/zk-soundness-audit.md); internal re-audits:
> [binding](../../research/zk-verifier-reaudit.md) +
> [membership/PoK](../../research/zk-membership-pok-reaudit.md)). Not FIPS-approved
> ([fips-posture](../../compliance/cryptoreview/fips-posture.md)). The opt-in `dual-leaf` value lane (`filter_value_dl_{int,f64,decimal}` datatype-class members + the `dispatch` fail-closed `(method × circuit)` matrix; sq-xojl/sq-cfmv/sq-2ezsx — plus the `xsd:boolean` lane, which adds NO member and rides `filter_value_dl_int` via its public `datatype_const`, sq-5xdlk, and `filter_value_dl_datetime`, ONE member serving BOTH the `xsd:dateTime` and `xsd:date` classes on a signed scaled-epoch value handle and separated by their public lane constants alone, sq-wz99x — whose `Z`-only hookable domain is enforced fail-closed HOST-side) carries the #769-accepted **INV-VL downgrade**, an open audit obligation (**CR-G8** / `sq-qhy4`) — no soundness/privacy claim (API in the SKILL). The OFF-by-default `extended-fragment` wave-1 lane (`CircuitId::PathReach` bounded-depth property-path member + the fail-closed `verifier::dispatch_fragment` routing gate over a `FragmentManifest`, routed end-to-end by `verifier::verify_fragment_manifest`; sq-3kd2g.6 / sq-h732x) runs an accepted extended query's sub-proofs through bb verification (routing stage-1 through `fragment_query`), and the disclosed-solution term binding — `verifier::bind_fragment_solution` (sq-1zf94) for the path predicate/endpoints (`PathReach` `pred_enc`/`src_enc`/`dst_enc`) + `VALUES` cells, and `verifier::bind_fragment_scans` (sq-qyfth) for the per-solution BGP scan-slot row selection (`BranchWitness::scan_rows`) — binds each disclosed solution variable to the terms the verifier re-derives from the disclosed solution + query text, with join coherence across atoms sharing a variable (fail-closed). The per-branch cross-graph Q6 non-bnode obligation AND the existential scan↔path / path↔path join coherence are now enforced by `verifier::bind_fragment_join_coherence` (sq-ygk6x) — every existential variable shared between a scan slot and a `PathReach` endpoint is bound by encoding-equality, and a multi-graph path (whose interior-chain non-bnode obligation the verifier cannot discharge) is refused fail-closed. Since `sq-nlulr`, `verifier::bind_issuer_attestations` records PATH-referenced committed graphs in the audit-#9 salt-uniqueness gate too (each path commitment now carries the same issuer-attestation requirement + distinct-salt record as a scan commitment), so a cross-graph scan↔single-graph-path join carries the SAME distinct-salt non-bnode discipline as a scan↔scan join, and an unattested / salt-colliding path commitment is refused fail-closed. The only residual now is BY DESIGN (not a non-bnode gap): existential (non-projected) path endpoint values stay hidden — so an accepted extended-fragment proof carries the same attestation + salt discipline as the flat path, still asserting no soundness/privacy property. Opus 4.8 — re-review when Fable returns. <!-- [OPUS-4.8] privacy-claims-allow: opt-in dual-leaf value lane + fail-closed dispatch matrix + OFF-by-default extended-fragment routing gate (verify_fragment_manifest, sq-h732x) with the disclosed-solution term binding landed for path endpoints/predicate + VALUES cells (bind_fragment_solution, sq-1zf94), BGP scan-slot row selection (bind_fragment_scans, sq-qyfth), AND the per-branch cross-graph Q6 non-bnode + scan↔path/path↔path join coherence (bind_fragment_join_coherence, sq-ygk6x, multi-graph path fail-closed); sq-nlulr CLOSED the #1684 path-graph-salt residual (bind_issuer_attestations now attests + salt-records PathReach commitments), leaving only the by-design hidden existential path endpoint value; INV-VL downgrade framed as an OPEN audit obligation; every lane explicitly asserts no soundness/privacy property; sq-qhy4 / CR-G8 -->

How-to + the covered/deferred matrix: [`skills/zk-query-proofs/SKILL.md`](../../skills/zk-query-proofs/SKILL.md).
Benchmarks (gate counts, timing): [`bench/zk-compose/`](../../bench/zk-compose).
Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
