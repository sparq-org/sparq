# SPARQL-RL syntax (opt-in)

Enable `sparq-shacl`'s default-off `sparql-rl` feature to call
`sparq_shacl::sparql_rl::parse(source, base) -> Result<Document<'_>, ParseError>`.
The base must be an absolute IRI. This is a syntax parser only: it does not
evaluate rules, check variable well-formedness or compute stratification.
It is independent of `shacl-af` and adds no dependencies or validation hooks.

`Document.statements` preserves BASE, PREFIX, VERSION, IMPORTS, DATA and RULE
statements in source order. `Statement::Rule` contains a `Rule` with an optional
name, head template, `base_only` flag and ordered `Element` values (triples,
FILTER, SET assignment, NOT/NOT DATA, GROUP). A GROUP contains keys, its inner
body and `Aggregate` assignments. Source slices borrow the input and preserve
delimiters and spelling; replay declarations in order when interpreting them.
IMPORTS is retained without loading anything. Prefixes must be declared before
use; BASE and prefix redefinitions apply from their declaration onward.

```rust,ignore
use sparq_shacl::sparql_rl::{parse, Statement};

let document = parse(
    "PREFIX : <http://example/> RULE { ?x :q ?y } WHERE { ?x :p ?y }",
    "http://example/rules.srl",
)?;
assert!(matches!(&document.statements[1], Statement::Rule(_)));
# Ok::<(), sparq_shacl::sparql_rl::ParseError>(())
```

`ParseError` carries `kind`, byte `offset`, one-based `line` and character
`column`, and `message`. `ErrorKind::Syntax` indicates syntax rejection;
`ErrorKind::Limit` indicates the parser's source-size or nesting bound was
exceeded. The parser accepts up to 1 MiB of source and 64 recursive grammar
levels. A limit error does not establish that the syntax is invalid. The grammar
is pinned to data-shapes `04ecbb4`; see the [design](../../../research/sparql-rl.md)
for the manifest census and the proposed evaluation model.
