<!-- [OPUS-5] sq-i6du2.8 (epic sq-i6du2, #1613) — 🤖 SPARQ agent. -->
# Semantics mapping — sparq's ODRL path ↔ ODRE inputs

The notes this lane's honesty rests on. The comparison is only meaningful to the extent
that both systems were asked the same question; everywhere they could not be, this file
says so, and the harness reports the case as out-of-scope rather than scoring it.

Design authority: [`research/ac-query-benchmark.md`](../../../research/ac-query-benchmark.md)
§2.3, §2.5, §4.2 (bead B8). Consumer: the `odrl-policy-bridge` paper's §5.3 study.

## 1. The two interfaces are not the same shape

| | sparq (`sparq-policy`) | ODRE (`pyodre`) |
|---|---|---|
| Entry point | `evaluate(&Policy, &Request) -> Decision` | `ODRE().enforce(policy) -> {action: result}` |
| Request | first-class: party, action, target, context values, membership evidence | **none** — `enforce` takes a policy and an interpolation dict |
| Rule selection | action ⊑ rule action, target match, assignee match, then constraints | every `odrl:permission` / `odrl:prohibition` / `odrl:obligation` in the graph |
| Prohibition | deny-overrides: a matching prohibition denies outright | evaluated exactly like a permission; a satisfied prohibition contributes positively |
| Constraint discovery | `odrl:constraint` → constraint node (ODRL 2.2) | `odrl:leftOperand`/`operator`/`rightOperand` read **directly off the rule node** |
| Constraint context | supplied by the request (`.at`, `.for_purpose`, `.with`) | resolved by evaluating a python function (`odrl_dateTime()` → `datetime.now()`) |
| Unknown vocabulary | evaluated as an unprovable dimension (fail-closed) | raises `Unknown URI` unless a prefix is registered |

The first row is the whole difficulty: **ODRE answers "does this policy hold right now",
sparq answers "may this party do this".** Every mapping decision below follows from
trying to put those two questions on the same footing without pretending they are one.

## 2. Three encodings, because the encoding IS a result

Reporting a single number here would hide the choice that produced it, so every case is
run three ways and all three are reported.

- **`standard`** — the ODRL 2.2 N-Triples exactly as sparq consumes them. This is the
  bytes-identical comparison. Because ODRE's constraint query does not reach a
  `odrl:constraint`-linked node, constraints are invisible to it under this encoding and
  rules fire unconditionally.
- **`odre-native`** — `standard` plus the two input normalisations pyodre's evaluator
  requires: constraint properties inlined onto the rule node, and timezone-naive
  `xsd:dateTime` lexical forms (pyodre's `cast_dateTime` uses `%Y-%m-%d %H:%M:%S` and
  raises on an offset-aware literal). The instant is unchanged.
- **`projected`** — `odre-native` plus harness-side **request binding**: rules whose
  `odrl:assignee` does not bind the requesting party, or which do not carry the requested
  action, are removed before ODRE sees the policy. This is the charitable encoding — it
  performs, outside ODRE, the step ODRE's API cannot express — and is the **headline**.

## 3. Request mapping

The corpus's request IR (`sparq_acbench::Request`) carries agent / client / resource /
mode and nothing else, so the harness supplies the remaining dimensions from the corpus's
own pinned constants — the same ones the by-construction oracle assumes.

| Dimension | sparq | ODRE |
|---|---|---|
| party | `Request::by(agent)` | not modelled; used by the harness in `projected` binding |
| action | `Request::new(acl#Read\|Write\|Control)` | compared against the rule's `odrl:action`; namespace registered via `add_prefix_mapping` |
| target | `Request::on(resource)` | not modelled (see §5, target binding) |
| instant | `Request::at(EVAL_INSTANT)` | `odrl_dateTime()` rebound to the same instant |
| purpose | `Request::for_purpose(GRANTED_PURPOSE)` | left operand declared unsupported → out-of-scope |
| count | `Request::with(odrl:count, 1)` | left operand declared unsupported → out-of-scope |
| device/client | `Request::with(odrl:systemDevice, client)` | left operand declared unsupported → out-of-scope |
| membership | `with_party_memberships(edges)` | applied by the harness in `projected` binding |

**Membership evidence.** `sparq-policy` matches a `odrl:PartyCollection` assignee only
against membership the request supplies — it never infers it. The harness therefore
reads the generator's exported `group_memberships` map and maps each group to the
compiled policy's `PartyCollection` IRI. It supplies the identical set to both sides,
plus `odrl:All` and, for a non-empty agent,
`odrl:AllAuthenticated`.

**Pinned constants.** `EVAL_INSTANT` and `GRANTED_PURPOSE` duplicate `pub(crate)`
constants in `sparq_acbench::oracle`. The exporter asserts both against the oracle's
observable behaviour on startup (an in-window intent must be admitted, an expired one
refused; the granted purpose admitted, any other refused), so drift aborts the export
instead of silently re-dating the corpus.

## 4. Harness interventions (each one a declared deviation)

Recorded per case in the report's `harness_interventions`, never left implicit.

1. **clock-pinning** — `odrl_dateTime()` is rebound to the corpus's pinned instant via a
   `PythonInterpreter` subclass. Without it every temporal decision would depend on the
   calendar date of the run and the two systems would not be answering the same question.
2. **prefix-registration** — the sparq acl action namespace is registered through ODRE's
   documented `add_prefix_mapping` extension point; without it every case raises
   `Unknown URI`. It adds no behaviour: no `acl_*` function exists, so ODRE reports the
   action as unsupported-but-permitted.
3. **constraint-inlining** (`odre-native`, `projected`) — see §2.
4. **tz-naive-datetime** (`odre-native`, `projected`) — see §2.
5. **request-binding** (`projected` only) — see §2.
6. **target-binding-not-performed** — declared *absence* of an intervention, so it cannot
   be mistaken for an equivalence (§5).

## 5. What is NOT comparable, and why

- **`odrl:purpose`, `odrl:count`, `odrl:systemDevice`** — declared unsupported by ODRE's
  own capability table (`odre-capabilities.json`). Cases using them are reported
  out-of-scope-for-ODRE. That is a scope statement about the comparison, not a defect
  finding about ODRE.
- **Multi-constraint rules** — a rule bearing two or more constraints is
  **unrepresentable** for ODRE and is refused rather than encoded. `ODRE._constraints`
  selects `?operator`, `?left_operand` and `?right_operand` independently from the rule
  node, i.e. their cartesian product. A two-bound temporal window inlines to four
  conjuncts including `dateTime >= end` and `dateTime < start` — unsatisfiable at every
  instant. Encoding it anyway would hand ODRE a policy that denies by construction and
  then score that denial. **Measured on pyodre 1.0.6, not inferred.** Consequence worth
  stating plainly: the corpora's two-bound retention/embargo windows — the constraint
  shape U3/U4 exist to stress — are outside what this ODRE version can represent, and the
  report flags a run whose comparable set never exercised constraint evaluation as WEAK
  EVIDENCE.
  - *Rejected alternative:* ODRE's `time:between` extension operator could express a
    window as one constraint, but it reads `datetime.now()` directly and cannot be
    pinned, so the comparison would stop being deterministic. It is also an ODRE-specific
    vocabulary, not ODRL 2.2.
- **Target binding** — not performed, because the acbench ODRL compiler attaches
  `odrl:target` at the policy node rather than per rule, and each case's policy is
  already scoped to the requested resource by construction.
- **Timing** — out of scope for this lane entirely (design record §2.5: agreement first,
  timing second). No wall-clock number is produced.

## 6. Divergence classes

Every disagreement must match a sourced entry in `known-divergences.json` or the run
fails (exit 3). The three classes:

- **mapping-gap** — the two systems were not asked the same question: the difference is
  attributable to the input mapping or to a capability one side does not model at all
  (e.g. ODRE having no request notion).
- **semantics-gap** — both read the same policy and disagree about what ODRL *means*; a
  documented, defensible difference of reading (e.g. deny-overrides vs a prohibition that
  fires like a permission). This lane does not adjudicate which reading is right.
- **implementation-bug** — neither of the above: an implementation does not do what its
  own specification or documentation says.

Each entry declares its `evidence` as `source-read` (a prediction from reading the pinned
implementation's source) or `observed`. The report's `ledger_matches` block records how
many times each entry actually fired, so an unconfirmed prediction stays visibly
unconfirmed instead of being promoted by hand.

## 7. Standing caveat

Nothing in this lane establishes that either system is correct. A PASS means the two
agreed on the comparable cases at the pinned instant under the declared interventions, or
that every difference matched a pre-registered, sourced explanation. No conformance,
soundness or security property of sparq or of ODRE is claimed or tested.
