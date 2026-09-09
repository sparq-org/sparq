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
