# Version-pinned EBV rules

[GPT-6] `QueryBudget.ebv_semantics` selects effective boolean value rules only.
`None` uses a recognized query `VERSION` announcement, otherwise
`EbvSemantics::Rec2013`. `Some(Rec2013)` explicitly pins the published
[21 March 2013 Recommendation §17.2.2](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#ebv).
`Some(Draft20260912)` selects the EBV change in the
[12 September 2026 Working Draft §17.2.3](https://www.w3.org/TR/2026/WD-sparql12-query-20260912/#ebv).
Neither option certifies full SPARQL 1.2 syntax or semantics.

Invalid raw numeric or boolean literals have false EBV in REC 2013, and an
ordinary expression error in the pinned draft. Thus `!!"z"^^xsd:boolean`
projects false in the former and an unbound cell in the latter. FILTER drops
both false and error; IF, NOT, AND, OR and COALESCE retain their normal error
rules. Numeric lexical validity, exact zero/NaN classification, term identity
and whole-query numeric/temporal capacity are separate unchanged contracts.

`PreparedQuery::parse` retains all announcements in source order. Labels `1.1`, `1.2`
and `1.2-basic` select the corresponding EBV rule; unsupported labels and
announcements requiring incompatible EBV rules are rejected. Repeated `1.2`
and `1.2-basic` labels are compatible. An explicit API option that
contradicts the announcement is rejected before evaluation. The draft
[version-label definitions](https://www.w3.org/TR/2026/WD-sparql12-query-20260912/#syntaxVersionAnnouncement)
cover more than EBV; this engine setting does not enforce every restriction
attached to a label. `PreparedQuery::from_query_with_versions` preserves metadata
when an integration first wraps parsed algebra. `PreparedQuery::with_query`
retains that metadata across a structural rewrite. Bare `From<Query>`/`into_query` carry
algebra alone; callers must retain announcement metadata separately.

Every query entry point installs its own selection. The executor copies it
once into its local vocabulary, inherited by GRAPH/EXISTS and shared read-only
with Rayon workers. Nested public engine invocations use their own option or
default and restore the outer query on return/unwind. Per-row EBV does not
read a global or thread-local selector. UPDATE and the PATHS extension remain
REC 2013 only: they reject other VERSION labels; budgeted UPDATE also rejects
an explicit draft EBV option. UPDATE protocol rewrites use
`parse_update_rec2013` before discarding parser metadata. The legacy vendored
parser still accepts syntax independently via `parse_update_with_versions`;
syntax acceptance does not establish an executable dialect.

Custom-aggregate parsing, inline OVER rewriting and text/structured EXPLAIN
ANALYZE preserve the same announcements. The opt-in engine result cache includes resolved EBV rules in
its key. Use `ResultCache::get_or_eval_prepared` to preserve an announcement;
the legacy algebra-only method has no announcement metadata and uses the
explicit budget option or REC 2013. Contradictions reject before cache lookup.

The proof evaluator explicitly pins REC 2013 and rejects contradictory query
announcements. Its existing serialized dialect/request identity stays unchanged.
The W3C runner selects the pinned draft for `sparql12` and `sparql12/` query suites and REC
2013 for earlier suites; expected results and conformance floors are untouched.
New native/model and actual-guest definitions require fresh execution under the
combined source; historical receipts do not attest this change.

Public Python ASK/SELECT-count, LWS/Solid dataset rewrites, geo/text/vector rewrites,
and SHACL pre-binding retain query announcements. SHACL retains its existing
lenient ill-formed-shape policy when labels cannot be resolved. HTTP clients
must announce `VERSION "1.2"` (or `1.2-basic`) to select draft EBV; unannounced
queries use REC 2013. Service-description version IRIs describe available query
syntax, not a full draft-conformance guarantee or UPDATE dialect selection.
