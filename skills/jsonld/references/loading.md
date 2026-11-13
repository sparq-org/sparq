# Loading JSON-LD and HTML documents

For trusted local HTML fixtures, enable the default-off feature:

```toml
[dependencies]
sparq-jsonld = { version = "0.1", features = ["html"] }
```

```rust
use sparq_jsonld::{expand, FsLoader, Json, JsonLdOptions};

let loader = FsLoader::new().map_prefix("https://example.test/", "fixtures/");
let mut options = JsonLdOptions::default();
options.extract_all_scripts = true;
let document = Json::Str("https://example.test/page.html#details".into());
let expanded = expand(&document, &options, &loader)?;
```

The fragment selects one JSON-LD script even though all-script extraction is enabled.
Without a fragment, that option merges every matching script. Scripts inside HTML
comments, raw-text elements, textarea content or template contents are not selected.
Foreign-namespace script elements are excluded from HTML script selection. Script text is parsed
as JSON without stripping comment wrappers or decoding HTML character references.
XHTML instead uses XML text, references and CDATA.

Applications can implement `DocumentLoader::load_document(&self, url)` and return a
`RemoteDocument`. Set `document_url` to the final resource URL and `content_type` to its
media type, with optional parameters. Set `context_url` only for applicable context
links. The loader handles status errors, redirect and alternate selection, and multiple
context link errors; the processor does not issue HTTP requests. Missing content type
retains the existing JSON-LD default.

The processor resolves a relative `context_url` against the final document URL and
processes it after `expand_context`. Explicit `options.base` changes identifier resolution
while remote-context references still use their document URL. HTML's first base element
resolves against the supplied base fallback and becomes the extracted document's base.
Compaction and compacted flattening use the final document URL for relative output when
there is no explicit base and `compact_to_relative` is enabled.

`FsLoader` is intended for trusted fixtures. It ignores URL query and fragment parts
when selecting a file and retains the supplied URL as document metadata. It does not
emulate HTTP response headers; the conformance harness has a separate manifest loader
for that purpose.
