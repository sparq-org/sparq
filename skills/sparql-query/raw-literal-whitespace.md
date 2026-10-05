# Raw literal validity and string construction

[GPT-6] RDF literal lexicals are preserved, including ill-typed terms. Numeric and
boolean interpretation checks the raw lexical against its datatype. XML boundary
whitespace is not removed from an already typed literal. By contrast, a constructor
such as `xsd:integer(" 1 ")` first normalizes its string input as specified by XPath.

[RDF 1.1 Concepts §5](https://www.w3.org/TR/rdf11-concepts/#section-Datatypes)
explicitly defines boolean lexical space as `true`, `false`, `1`, and `0`.
The numeric lexical grammars likewise exclude boundary whitespace:
[XSD 1.0 Part 2](https://www.w3.org/TR/2004/REC-xmlschema-2-20041028/#decimal)
and [XSD 1.1 lexical space](https://www.w3.org/TR/2012/REC-xmlschema11-2-20120405/#lexical-space)
distinguish lexical validation from a host's pre-lexical preprocessing.
The shared numeric facet contract retains modern XSD 1.1 unsigned signs, including
`+1` and `-0`; this correction adds no legacy datatype mode.

[XPath 2.0 F&O §17.1.1](https://www.w3.org/TR/2010/REC-xpath-functions-20101214/#casting-from-strings)
applies whitespace normalization when casting from a string or untyped atomic
value. The four XML whitespace characters are accepted around constructor input;
NBSP, vertical tab and internal numeric whitespace remain invalid. Raw typed
identity casts do not receive string preprocessing.

The [published SPARQL 1.1 EBV rule](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#ebv)
returns false for an invalid numeric or boolean lexical. Thus
`!!"z"^^xsd:boolean` is false. Arithmetic, comparison against a different value and
typed constructor use can instead produce expression errors; `COALESCE` may handle
those ordinary errors. None of these cases is an arithmetic-capacity failure.
Projection, `STR`, `DATATYPE` and `sameTerm` preserve the original lexical identity.

The SPARQL 1.2 EBV definition changes invalid-literal behavior to an error. A 1.2
suite expectation of unbound for the same double-negation example is a version
difference, not an incorrect 1.2 fixture. The initial proof profiles select the
published 2013 operator contract, and this change does not silently select 1.2.

## Correction history and validation scope

Earlier native cache-alignment tests treated `" 5 "^^xsd:integer` and
`" 7 "^^xsd:decimal` as numeric values. Those expectations conflated XML
preprocessing with raw RDF datatype membership. Their exact old assertions remain
in Git history. The two stored-arithmetic/comparison `" 5 "` acceptance assertions
in `builtin_edges.rs` also change to rejection; the string-cast control stays true.
This change corrects the cache assertions to misses and adds independent
raw-validation expectations alongside the substrate/reasoner parity tests.
The existing builtin and EBV fixture inventories are retained unchanged. The new
32-case raw-literal matrix has manually authored complete expected results.

The engine matrix covers constant, VALUES, BIND and stored terms under ordinary
and strict budgets, with dense and compressed dictionaries. Core migration tests
synthesize the old v2 padded-numeric cache entry and verify that v3 regeneration
rejects its numeric value without rewriting old bytes or removing any RDF term.
The v3 numeric sidecar changes derived-cache compatibility only. Legacy archives
need a dictionary scan until saved to a separate current archive; no performance
neutrality is claimed for that migration.

Independent engine observations are supplementary: PyOxigraph 0.5.11 rejects the
padded boolean EBV example, while RDFLib 7.6.0 reports true; their tested string
constructor behavior also differs from the pinned XPath normalization rule.
These are interoperability observations, not sources of the expected results.
Actual guest execution and source-bound receipts remain separate evidence from
native regression tests and runner definitions.

V1 and V2 separately consume all 32 raw-literal goldens through native and actual-guest runner definitions. Shared source and native success do not establish execution of a new guest image.
