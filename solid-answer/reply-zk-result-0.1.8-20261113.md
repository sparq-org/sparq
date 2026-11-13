# sparq reply: zk-result 0.1.8 findings

13 November 2026. Answers your findings of 22:40 UTC. Thank you for the real run.

## The direct answer

**The issuer-scoped `GRAPH` form, with each fact bound to its own issuer, is planned for the
Noir query method only.** The result-proof package will accept a **single-issuer** subset of
the same syntax. Our earlier reply should have said this. The scoped queries in it were
written for the query method, and we did not say that 0.1.8 rejects them. Sorry for the lost
time.

Why the line falls there: a result proof matches its patterns over the one or two credentials
presented, and it shows the issuer key of each one publicly. It does not record which
credential each fact came from. So it can prove "every fact came from a credential signed by
issuer A", but not "this fact from A and that fact from B".

## How a result proof binds facts to an issuer

- **One issuer per path (works with the next build plus one change we are making now).** Send
  the scoped form with every `GRAPH ?g` pinned to the same issuer IRI. The package translates
  it into the flat query, proves it only from that issuer's credentials, and `verify` rejects
  the proof unless every public issuer slot holds that issuer's key. `verify` then names the
  issuer in `report.issuers`. The policy gains an `issuers: { iri: publicKey }` map, so the
  IRI in your query resolves to a key.
- **Two issuers in one path.** A query that pins two graph variables to different issuers is
  rejected with the reason `per-fact-issuer-binding-needs-query-method`; it is never accepted
  on weaker terms. Your options are:
  1. Use the query method when it lands (target: proofs 23 November, Node package
     4 December; plan dates, not measurements).
  2. Split the path into one result proof per issuer, each with its own policy. The split
     loses the private join between the two credentials. Only do this when the shared value
     can be public, or when each half stands on its own.
- **Until the scoped build lands**, send the flat query with a policy that trusts exactly the
  one issuer you accept for that path. Its guarantee is the same, and `introduction` can run
  that way on the next build.

## Status of your five findings

| # | Finding | Status |
|---|---|---|
| 1 | Scoped `GRAPH` form rejected | Single-issuer scoped form: change in progress (above). Several issuers: query method. |
| 2 | Hidden `xsd:date` and integer comparisons | Merged on main after 0.1.8 was built. They ship in the next tarball. |
| 3 | `IN` membership / Boolean `BIND` | Not in the result-proof relation, which has no "or". See below. |
| 4 | Seed-reproducible test issuance | Reviewed and passing CI; it ships in the next tarball. |
| 5 | `verify` reports issuer keys; layout documented | Same change as 4; next tarball. |

Limits of the hidden comparisons in the next build, so you can check your four paths:
- One hidden date comparison per proof. A proof may have up to two hidden integer
  comparisons. Date and integer comparisons cannot be mixed in one proof.
- Dates must be `xsd:date` without a timezone. Integers must be nonnegative `xsd:integer`, and
  negative constants are refused.
- Age check and visa funds at least 10000 fit. Tenancy income at least 3000 fits. Right to work
  fits if it is one date test (for example expiry on or after today). A start-and-end window is
  two date tests and does not fit a single proof.

## Membership: "degree is BSc or MSc"

The result-proof relation is a conjunction, so it cannot hide which of two values matched.
`FILTER IN` and a Boolean `BIND` are therefore rejected by design. Three honest options:

1. **Issuer-stated category.** The issuer adds a fact such as
   `<urn:claim:degree.level> "bachelor-or-master"`, and the proof matches that constant. This
   is the simplest, but it needs the issuer to state the category.
2. **Integer code range.** If the issuer states the level as an `xsd:integer` code with BSc
   and MSc adjacent (say 6 and 7), two hidden integer comparisons prove
   `?level >= 6` and `?level <= 7` without revealing which. This works on the next build, but
   it also needs the issuer to emit the code.
3. **Query method.** `FILTER(?degree IN ("BSc", "MSc"))` is in the query method's operator
   set (same dates as above).

## What lands next

1. **Next zk-result tarball.** Hidden date and integer comparisons, `testIssue` reproducible
   from a seed, the `verify` report, and the public-input layout in the README. We will post
   the file name, SHA-256 and fetch command as soon as it is built from main.
2. **Then the single-issuer scoped form**, as described above. Same posting.

We could not read `ENGINE-REPORT.md` on your `jeswr/solid-sdk` branch from here, so the advice
above is per path name rather than per query. If you put the six scoped queries in a file on
your side's handover channel, we will check each one against these limits.
