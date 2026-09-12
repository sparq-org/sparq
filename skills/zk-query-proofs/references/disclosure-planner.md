# Disclosure planning and selected result witnesses

<!-- [GPT-6] zkp-3: host planning surface, with no cryptographic acceptance claim. -->

`sparq_zk_compose::planner` derives a strict positive query fragment from the
existing SPARQL parser and selects a successful witness for each released result.
It is a **host planner**, not a proof verifier. Its output remains research-grade
and is not externally audited. A backend must independently bind the public query
and released result to credential authentication, membership, joins, and hidden
predicates before accepting a presentation.

## Admission and result contract

Use `DisclosureQuery::parse(query_text)` for SELECT DISTINCT or true ASK over a
nonempty positive BGP, JOIN, and conjunctive directly bound integer FILTERs. The
parser rejects bag SELECT, REDUCED, LIMIT/OFFSET, ORDER BY, dataset clauses,
expression projections, subqueries, aggregates, GRAPH, SERVICE, nonpositive
operators, query blank-node syntax, and unsupported FILTER expressions. A FILTER
variable must be bound in its own input scope; a variable introduced only by a
sibling join is rejected.

An integer FILTER uses a canonical nonnegative `xsd:integer` operand and bound
representable as `u64`. Other numeric types and lexical variants are outside this
predicate relation. The same term can still appear in an ordinary BGP without an
integer FILTER. RDF identity, including literal lexical form and datatype, is
preserved for pattern matching and DISTINCT; FILTER uses numeric comparison.

Admission has conservative resource limits before any witness search. Query text
is limited by `MAX_DISCLOSURE_QUERY_BYTES`; raw ASCII punctuation, including
punctuation inside public literals and IRIs, is limited by
`MAX_DISCLOSURE_QUERY_PUNCTUATION`. This preparse fuel counter does not tokenize or
assign SPARQL meaning. It can reject a simple query containing unusually
punctuation-heavy terms. The existing parser additionally limits syntactic
nesting. The planner walks graph and expression ASTs iteratively with bounded
work, and limits both parsed and programmatically constructed shapes using
`MAX_DISCLOSURE_PATTERNS` and `MAX_DISCLOSURE_FILTERS`. These are host safeguards,
not proof circuit capacities.

Both selection paths reject more than `MAX_DISCLOSURE_CREDENTIALS` supplied
credentials using a slice-length check before validation, statistics, or search.
Empty and ineligible credentials count toward this cap. This independently bounds
empty-graph traversal and the optimizer's input statistics prepass, since empty
graphs do not consume candidate-triple fuel.

`plan_disclosure(&query, &credentials, &released, PlannerLimits::default())` takes
`GraphCommitment` values and released rows represented as
`BTreeMap<String, oxrdf::Term>`. Each row must have exactly the projected variable
names, without question marks. Duplicate rows and projected blank nodes are
rejected. Every row must have a compatible full-pattern witness that satisfies all
FILTERs. Additional valid answers may remain unreleased: this is answer support,
not complete-wallet evaluation. An empty SELECT release asserts no positive
answer. A true ASK is represented by exactly one empty binding; false ASK is
unsupported.

## Private witness preparation

The planner searches in credential and canonical leaf order, backtracking over
join candidates and ignoring candidates that fail FILTERs. It chooses one witness
per released row; it does not enumerate every answer or claim minimum total proof
cost. `PlannerLimits` bounds pattern depth, result count, and total candidate
attempts. Exhaustion returns an error rather than a partial plan. Backend circuit
capacities are separate restrictions that must also be checked.

Each `plan.rows[row].witnesses[pattern]` identifies a canonical leaf using
`MembershipRef { credential, leaf }`. The top-level `memberships` list deduplicates
these pairs across patterns and results. `authentication` contains each selected
credential once and omits unused credentials. Deduplication is by credential
identity and leaf index, never by plaintext value across unrelated credentials.

**Witness locations, plan metrics, and explanations are prover-local.**
`DisclosurePlan` has no serialization implementation. Copying its indices or
counts into a public manifest would disclose credential attribution or witness
structure. The plan does not authenticate the supplied graph commitments and
must never serve as a cryptographic acceptance verdict.

Blank-node matches carry credential scope. The same canonical blank-node label in
different credentials is not a shared RDF identity. Hidden joins inside one
credential are admitted by the host planner; a backend may enforce a narrower
domain. IRIs and literals retain their RDF identity across credentials.

## Disclosure and proof work

`public_slots` contains only query constants and actually released terms.
`disclosed_triples[pattern]` is present only when every slot is determined by that
public information. A hidden salary remains absent even when the person's IRI is
released. A fully determined triple still retains its selected membership and
credential authentication obligations: public knowledge of a value does not
authenticate its origin.

`FilterObligation::PublicCheck` allows predicate computation on the authenticated
released term. `HiddenPredicate` identifies an authenticated operand slot that a
backend must constrain. `IdentityObligation` connects every repeated hidden
variable occurrence to its first occurrence, including repetitions within one
triple. Backends must enforce RDF term identity, including blank-node scope, on
these links.

`DisclosureQuery::variables()` gives deterministic first-occurrence variable
ordering for backend IDs. `canonical_integer` and `integer_comparison` provide
the same narrow numeric semantics to a verifier. Verifiers reparse their public
query using `DisclosureQuery::parse`; they never trust the public fields of a
prover-supplied `DisclosureQuery` instance.

Host regression coverage lives in
`crates/sparq-zk-compose/tests/disclosure_planner.rs`. It covers public versus
hidden disclosure, mixed passing/failing candidates, join backtracking, shared
memberships, independent credentials, blank-node scope, exact release domains,
unsupported query operators, and bounded search. These tests do not run a prover
and provide no cryptographic assurance.

## Optional joint witness optimization

<!-- [GPT-6] zkp-8: bounded structural selection, no calibrated latency claim. -->

`optimize_disclosure` and `optimize_disclosure_admitted` are explicit alternatives
to the unchanged first-success selection policy. They keep the exact query and
released rows fixed and jointly choose witnesses across all rows. The objective
is lexicographic: minimize unique credential authentication obligations first,
then unique credential/leaf memberships. It measures structural work only; it is
not a calibrated estimate of proving latency or proof size.

The admitted variant takes the same deterministic backend restriction as
`plan_disclosure_admitted`. No query check is waived. Original credential and
canonical leaf indices are retained, and committed graphs are never trimmed.
`OptimizationLimits.max_authentications` additionally restricts joint assignments
to a backend's distinct-credential capacity. Optimality and infeasibility are
always relative to these admission and capacity restrictions.

Search streams candidates in deterministic released-row, pattern, original
credential and canonical-leaf order. It maintains reference counts for selected
credentials and memberships and prunes prefixes which exceed capacity or cannot
improve the incumbent objective. It does not store an exponential list of each
row's possible witnesses. Equal objective values keep the first assignment in
that traversal order.

`OptimizationLimits.planner` supplies the pattern, result and global candidate
budget. `max_pattern_occurrences` bounds the joint result-by-pattern positions;
an absolute implementation cap also bounds recursive stack and live partial
bindings. The `OptimizationReport` returns fixed input sizes, candidate attempts,
successively improved complete assignments, and pruned prefixes. Counts and selected
witnesses remain private local diagnostics.

The completion flag has three meanings:

- `Optimal`: search established the minimum structural objective in the admitted
  space; `plan` contains a complete feasible assignment.
- `Infeasible`: exhaustive search found no admitted assignment for the fixed
  released rows; `plan` is absent. This is a host search result, not a proof of
  query absence.
- `BudgetExhausted`: optimality and infeasibility are unestablished. `plan` may
  contain the best complete feasible assignment seen so far, or be absent if none
  was reached. It never contains only some requested rows.

A caller may explicitly use an exhausted search's feasible plan, subject to normal
proof construction and verification, but must retain its incomplete-optimization
status. An exhausted search without a plan must not be reported as an impossible
query. The baseline preparation path does not silently switch policies.

`tests/disclosure_optimizer.rs` compares the complete objective and deterministic
tie assignment against an independently implemented exhaustive oracle across
small datasets. Further regressions cover shared credentials, membership reuse,
capacity restrictions, admission, exact rows, RDF identity, and forced exhaustion.
These are structural host tests; they do not measure prover runtime or audit
cryptographic correctness.


## Canonical signed-integer admission

[GPT-6] `planner::signed::SignedDisclosureQuery::parse` separately admits the same
positive query shape with canonical signed `i64` comparison bounds. Use
`plan_signed_disclosure[_admitted]` or `optimize_signed_disclosure[_admitted]` with
that type. The unsigned `DisclosureQuery::parse`, planning functions and proof
verifier keep their existing nonnegative `u64` semantics.

`canonical_signed_integer` accepts only the exact canonical `xsd:integer` token
within the signed range: zero is `0`; negatives have one leading minus; no leading
plus, leading zero, negative zero, whitespace or numeric datatype substitution is
normalized. Other valid XML Schema spellings are outside this admitted profile.
The parser rejects noncanonical and out-of-range public bounds. A noncanonical or
out-of-range private operand cannot satisfy a planned predicate.

The signed query wrapper exposes `kind`, `projection`, unchanged `patterns` and
`filters` with actual signed bounds. Its internal order-preserving bias is not an
unsigned query instance available to callers. Both signed search paths apply the
same RDF identity, backend admission, resource limits and proof-obligation rules;
original committed graphs, literal spellings and membership references are never
rewritten. Host selection is not signed-predicate proof verification.

[GPT-6] The separate Noir `result_signed_integer` core gadget now reconstructs
the exact canonical signed token from a private order-preserving unsigned value.
It reuses the existing literal hash and comparison functions, handles the signed
minimum without signed negation, and uses one fixed capacity for both signs and
all admitted lexical lengths. Its unit checks cover boundaries, ordering across
zero, noncanonical spellings and type substitution. This is a tested core gadget;
no signed successful-result preparation or verification entry point is exposed by
this checkpoint, and these unit executions are not genuine result proofs.
