//! [FABLE-5] (sq-lsp7k.8, epic sq-lsp7k) MATERIALIZING tabular→RDF import — the `tabular`
//! subcommand: CSV **direct mapping** + the **R2RML materializing subset** over CSV logical
//! tables. sparq's counter-story to SQL virtualization platforms is *import fast, reload fast*:
//! CSV rows stream through a first-party RFC-4180 reader, are mapped row-by-row to N-Triples
//! chunks, and feed the existing parallel N-Triples ingest (`Graph::load_reader_parallel`) —
//! no whole-file buffering at any stage, and `.csv.gz`/`.csv.zst`/`.csv.bz2` reuse
//! `open_reader`'s transparent streaming decompression.
//!
//! **Direct mapping** (W3C Direct Mapping flavor, IRI rows instead of blank nodes):
//! - table name = the file stem (`people.csv.gz` → `people`), percent-encoded where needed;
//! - subject   = the `--template` IRI template (`{col}` + `{_row}` placeholders; default
//!   `<base><table>/row/{_row}`, `{_row}` = 1-based data-row number);
//! - predicate = `<base><table>#<column>`;
//! - object    = the cell as a literal, with datatype inference (`xsd:integer` / `xsd:decimal`
//!   / `xsd:double` / `xsd:boolean`, else a plain string; `--no-infer` keeps everything a
//!   plain string). An EMPTY cell is NULL: no triple.
//! - each row gets `rdf:type <base><table>` (override with `--class <iri>`, drop with
//!   `--class none`).
//!
//! **R2RML subset** (`--mapping <r2rml.ttl>`): logical tables are CSV files bound by
//! `rr:tableName` = file stem (or an explicit `name=path` positional). Supported:
//! `rr:logicalTable`/`rr:tableName`, `rr:subjectMap`/`rr:subject`, `rr:class`,
//! `rr:predicateObjectMap`, `rr:predicateMap`/`rr:predicate`, `rr:objectMap`/`rr:object`,
//! `rr:template` (IRI-safe percent-encoding of substituted values in IRI maps),
//! `rr:column`, `rr:constant`, `rr:termType` (IRI/Literal/BlankNode), `rr:datatype`,
//! `rr:language`, plus — [OPUS-5] (sq-u1z86) — **cross-CSV joins**
//! (`rr:parentTriplesMap` + `rr:joinCondition`/`rr:child`/`rr:parent`) and **named-graph
//! output** (`rr:graphMap`/`rr:graph`). Everything else in the `rr:` namespace — notably
//! `rr:sqlQuery`, `rr:sqlVersion`, `rr:inverseExpression` — is a LOUD error (fail-closed),
//! never a silent skip. An empty CSV cell is NULL per R2RML: the term map generates no term
//! and the triple (or the whole row, for a subject map) is skipped. Column-valued literals
//! stay plain strings (CSV's natural datatype) unless the mapping says
//! `rr:datatype`/`rr:language` — datatype inference is a direct-mapping convenience only,
//! never applied to R2RML output.
//!
//! **Joins** (`rr:parentTriplesMap`): a referencing object map is executed as a KEYED HASH
//! JOIN, not a nested scan — the parent CSV is pre-scanned ONCE into a
//! `join-key tuple → parent subjects` index, then the child table streams past it as usual.
//! Honest cost: the index is the one part of the pipeline that is NOT constant-memory (it
//! holds one key tuple + subject IRI per parent row); the child side still streams. SQL join
//! semantics on NULL: an empty join cell on either side matches nothing. At least one
//! `rr:joinCondition` is REQUIRED — the join-condition-free (cross-join) form is a loud
//! error rather than a guess.
//!
//! **Named graphs** (`rr:graphMap`/`rr:graph`): a graph map on the subject map scopes that
//! triples map's class + predicate-object triples; a graph map on a predicate-object map
//! scopes just its own; the two sets UNION, and an empty set (or the `rr:defaultGraph`
//! constant) means the default graph. The moment a mapping uses ANY graph map the emitter
//! switches from N-Triples to **N-Quads** and the load path becomes a DATASET load
//! (`Graph::load_dataset`, which preserves named graphs so `GRAPH ?g { … }` works). Honest
//! cost: that dataset load is whole-document, so the quad path buffers the generated
//! N-Quads in memory — `--out` still streams. A mapping with NO graph map keeps the
//! unchanged streaming N-Triples fast path (`load_reader_parallel`).
//!
//! **Row provenance** (`--row-provenance`, both modes): every generated subject also gets
//! `prov:wasDerivedFrom <base><table>/row/{_row}>` — the SAME row IRI the direct mapping's
//! default subject template produces, so a custom-template or R2RML subject stays linked to
//! the exact source row. Emitted into the row's graph(s), so it follows `rr:graphMap`.
//!
//! SQL-connection R2RML (virtualization — a deliberate non-goal; sparq's counter-story is
//! materializing import) and a GUI import wizard are explicitly OUT of scope (follow-ons
//! live in the bead tree under epic sq-lsp7k / sq-ixc3).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::time::Instant;

const RR: &str = "http://www.w3.org/ns/r2rml#";
/// `rdf:type`, pre-serialised as an N-Triples predicate — the class statement is emitted per
/// ROW, so its fixed terms are built once here, never per row.
const RDF_TYPE_NT: &str = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
/// The R2RML constant naming the default graph — a MEMBER of the graph set like any other
/// (next to a second graph map it adds the default graph, it does not cancel it).
const RR_DEFAULT_GRAPH: &str = "http://www.w3.org/ns/r2rml#defaultGraph";
/// Row-provenance predicate (`--row-provenance`) — W3C PROV-O, no invented vocabulary.
const PROV_WAS_DERIVED_FROM: &str = "http://www.w3.org/ns/prov#wasDerivedFrom";

// ---------------------------------------------------------------------------------------------
// Streaming RFC-4180 CSV reader
// ---------------------------------------------------------------------------------------------

/// Streaming RFC-4180 CSV record reader over any `Read` — quoted fields (embedded separators,
/// quotes-as-`""`, embedded newlines), CRLF or LF record ends, optional UTF-8 BOM, and a
/// configurable single-byte separator. STRICT + loud: a stray quote, a bare CR, an unterminated
/// quoted field, or non-UTF-8 bytes are errors, never silent coercions. Fully-empty lines are
/// skipped (they carry no record). Never buffers more than the internal 64 KiB block + the
/// current record.
struct CsvRows<R: Read> {
    src: R,
    buf: Vec<u8>,
    pos: usize,
    len: usize,
    sep: u8,
    bom_done: bool,
    pushback: Vec<u8>,
    eof: bool,
    failed: bool,
}

impl<R: Read> CsvRows<R> {
    fn new(src: R, sep: u8) -> Self {
        CsvRows {
            src,
            buf: vec![0u8; 64 * 1024],
            pos: 0,
            len: 0,
            sep,
            bom_done: false,
            pushback: Vec::new(),
            eof: false,
            failed: false,
        }
    }

    /// Next raw byte from the buffered source (`None` = EOF).
    fn raw_byte(&mut self) -> Result<Option<u8>, String> {
        loop {
            if self.pos < self.len {
                let b = self.buf[self.pos];
                self.pos += 1;
                return Ok(Some(b));
            }
            if self.eof {
                return Ok(None);
            }
            self.len = self.src.read(&mut self.buf).map_err(|e| format!("CSV read error: {e}"))?;
            self.pos = 0;
            if self.len == 0 {
                self.eof = true;
            }
        }
    }

    /// Next byte after one-shot UTF-8 BOM stripping at stream start.
    fn next_byte(&mut self) -> Result<Option<u8>, String> {
        if !self.bom_done {
            self.bom_done = true;
            let mut head: Vec<u8> = Vec::with_capacity(3);
            while head.len() < 3 {
                match self.raw_byte()? {
                    Some(b) => head.push(b),
                    None => break,
                }
            }
            if head.as_slice() != [0xEF, 0xBB, 0xBF] {
                self.pushback = head;
            }
        }
        if !self.pushback.is_empty() {
            return Ok(Some(self.pushback.remove(0)));
        }
        self.raw_byte()
    }

    /// `\r` must be followed by `\n` (RFC 4180 CRLF); a bare CR is a loud error.
    fn expect_lf(&mut self) -> Result<(), String> {
        match self.next_byte()? {
            Some(b'\n') => Ok(()),
            _ => Err("bare CR (a \\r not followed by \\n) in CSV input".into()),
        }
    }

    /// Parse one record. `Ok(None)` = clean EOF.
    fn record(&mut self) -> Result<Option<Vec<String>>, String> {
        #[derive(PartialEq)]
        enum S {
            Start,
            Unquoted,
            Quoted,
            QuoteQuote,
        }
        fn finish(fields: &mut Vec<String>, field: &mut Vec<u8>) -> Result<(), String> {
            let bytes = std::mem::take(field);
            fields
                .push(String::from_utf8(bytes).map_err(|_| "CSV field is not valid UTF-8".to_string())?);
            Ok(())
        }
        let mut fields: Vec<String> = Vec::new();
        let mut field: Vec<u8> = Vec::new();
        let mut any = false;
        let mut s = S::Start;
        loop {
            let Some(b) = self.next_byte()? else {
                return match s {
                    S::Quoted => Err("unterminated quoted CSV field at end of input".into()),
                    _ => {
                        if !any {
                            return Ok(None);
                        }
                        finish(&mut fields, &mut field)?;
                        Ok(Some(fields))
                    }
                };
            };
            any = true;
            match s {
                S::Start => {
                    if b == self.sep {
                        finish(&mut fields, &mut field)?;
                    } else {
                        match b {
                            b'"' => s = S::Quoted,
                            b'\n' => {
                                finish(&mut fields, &mut field)?;
                                return Ok(Some(fields));
                            }
                            b'\r' => {
                                self.expect_lf()?;
                                finish(&mut fields, &mut field)?;
                                return Ok(Some(fields));
                            }
                            _ => {
                                field.push(b);
                                s = S::Unquoted;
                            }
                        }
                    }
                }
                S::Unquoted => {
                    if b == self.sep {
                        finish(&mut fields, &mut field)?;
                        s = S::Start;
                    } else {
                        match b {
                            b'\n' => {
                                finish(&mut fields, &mut field)?;
                                return Ok(Some(fields));
                            }
                            b'\r' => {
                                self.expect_lf()?;
                                finish(&mut fields, &mut field)?;
                                return Ok(Some(fields));
                            }
                            b'"' => return Err("stray '\"' inside an unquoted CSV field".into()),
                            _ => field.push(b),
                        }
                    }
                }
                S::Quoted => match b {
                    b'"' => s = S::QuoteQuote,
                    _ => field.push(b),
                },
                S::QuoteQuote => {
                    if b == self.sep {
                        finish(&mut fields, &mut field)?;
                        s = S::Start;
                    } else {
                        match b {
                            b'"' => {
                                field.push(b'"');
                                s = S::Quoted;
                            }
                            b'\n' => {
                                finish(&mut fields, &mut field)?;
                                return Ok(Some(fields));
                            }
                            b'\r' => {
                                self.expect_lf()?;
                                finish(&mut fields, &mut field)?;
                                return Ok(Some(fields));
                            }
                            _ => {
                                return Err(format!(
                                    "unexpected byte 0x{b:02x} after the closing '\"' of a quoted CSV field"
                                ))
                            }
                        }
                    }
                }
            }
        }
    }
}

impl<R: Read> Iterator for CsvRows<R> {
    type Item = Result<Vec<String>, String>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            match self.record() {
                Ok(Some(rec)) => {
                    // A fully-empty line is not a record.
                    if rec.len() == 1 && rec[0].is_empty() {
                        continue;
                    }
                    return Some(Ok(rec));
                }
                Ok(None) => return None,
                Err(e) => {
                    self.failed = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// RDF term generation
// ---------------------------------------------------------------------------------------------

/// A generated RDF term, ready for N-Triples serialisation.
#[derive(Clone, Debug, PartialEq)]
enum GenTerm {
    Iri(String),
    Blank(String),
    Lit { value: String, datatype: Option<String>, lang: Option<String> },
}

/// N-Triples serialisation of one term.
fn nt_term(t: &GenTerm) -> String {
    match t {
        GenTerm::Iri(i) => format!("<{i}>"),
        GenTerm::Blank(l) => format!("_:{l}"),
        GenTerm::Lit { value, datatype, lang } => {
            let mut s = format!("\"{}\"", escape_literal(value));
            if let Some(l) = lang {
                s.push('@');
                s.push_str(l);
            } else if let Some(dt) = datatype {
                s.push_str("^^<");
                s.push_str(dt);
                s.push('>');
            }
            s
        }
    }
}

/// N-Triples literal escaping: `\` `"` `\n` `\r` `\t` plus `\u00XX` for other C0 controls.
fn escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// R2RML "IRI-safe" percent-encoding of a template-substituted value: keep iunreserved
/// (ALPHA / DIGIT / `-` `.` `_` `~` and non-ASCII), percent-encode every other byte.
fn iri_safe(v: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(v.len());
    for &b in v.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') || b >= 0x80 {
            out.push(b);
        } else {
            out.extend_from_slice(format!("%{b:02X}").as_bytes());
        }
    }
    // Kept bytes are the original (valid) UTF-8 sequences; escapes are ASCII.
    String::from_utf8(out).expect("iri_safe preserves UTF-8 validity")
}

/// Minimal absolute-IRI sanity check: a scheme, and none of the characters N-Triples /
/// RFC 3987 forbid raw inside `<...>`. Loud error, never a silently-broken output file.
fn check_iri(iri: &str) -> Result<(), String> {
    let absolute = iri
        .split_once(':')
        .map(|(scheme, _)| {
            !scheme.is_empty()
                && scheme.as_bytes()[0].is_ascii_alphabetic()
                && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        })
        .unwrap_or(false);
    if !absolute {
        return Err(format!("generated IRI {iri:?} is not absolute (no scheme; pass --base to resolve relative IRIs)"));
    }
    if let Some(bad) = iri.chars().find(|&c| (c as u32) <= 0x20 || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\')) {
        return Err(format!("generated IRI {iri:?} contains forbidden character {bad:?}"));
    }
    Ok(())
}

/// Deterministic, injective blank-node label from a value: ASCII alphanumerics kept,
/// every other byte (including `_`) becomes `_HH` (hex) — two distinct values can never
/// collide on a label.
fn blank_label(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for &b in v.as_bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else {
            out.push_str(&format!("_{b:02X}"));
        }
    }
    if out.is_empty() {
        out.push_str("_empty");
    }
    out
}

/// Direct-mapping datatype inference over a cell's lexical form. Returns the XSD datatype
/// IRI, or `None` for a plain string. Deliberately conservative: `INF`/`NaN` and exotic
/// numeric forms stay strings.
fn infer_datatype(v: &str) -> Option<&'static str> {
    if v == "true" || v == "false" {
        return Some("http://www.w3.org/2001/XMLSchema#boolean");
    }
    let unsigned = v.strip_prefix(['+', '-']).unwrap_or(v);
    if !unsigned.is_empty() && unsigned.bytes().all(|b| b.is_ascii_digit()) {
        return Some("http://www.w3.org/2001/XMLSchema#integer");
    }
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (unsigned, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some((i, f)) => {
            (!i.is_empty() || !f.is_empty())
                && i.bytes().all(|b| b.is_ascii_digit())
                && f.bytes().all(|b| b.is_ascii_digit())
        }
        None => !mantissa.is_empty() && mantissa.bytes().all(|b| b.is_ascii_digit()),
    };
    if !mantissa_ok {
        return None;
    }
    match exponent {
        None => {
            // No exponent: mantissa_ok + a '.' means xsd:decimal (pure digits handled above).
            if mantissa.contains('.') {
                Some("http://www.w3.org/2001/XMLSchema#decimal")
            } else {
                None
            }
        }
        Some(e) => {
            let e = e.strip_prefix(['+', '-']).unwrap_or(e);
            if !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit()) {
                Some("http://www.w3.org/2001/XMLSchema#double")
            } else {
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Compiled term maps
// ---------------------------------------------------------------------------------------------

/// One template piece: literal text, a column reference (by resolved index), or the
/// 1-based data-row number (`{_row}`, direct mapping only).
#[derive(Clone, Debug)]
enum TplPart {
    Text(String),
    Col(usize),
    Row,
}

/// Where a term's lexical value comes from.
#[derive(Clone, Debug)]
enum ValueSpec {
    Constant(GenTerm),
    Column(usize),
    Template(Vec<TplPart>),
}

/// What kind of term the value becomes.
#[derive(Clone, Debug, PartialEq)]
enum TermKind {
    Iri,
    Blank,
    LitPlain,
    LitTyped(String),
    LitLang(String),
    /// Direct-mapping convenience: per-cell datatype inference. Never produced from R2RML.
    LitInfer,
}

#[derive(Clone, Debug)]
struct TermSpec {
    value: ValueSpec,
    kind: TermKind,
}

/// [OPUS-5] (sq-u1z86) A referencing object map (`rr:parentTriplesMap` + `rr:joinCondition`)
/// compiled to a KEYED HASH JOIN: the parent table was pre-scanned once into
/// `join-key tuple → the parent subjects that key generates`, so the child table can stream.
#[derive(Clone, Debug)]
struct RefJoin {
    /// Child-row column indices of the join conditions, in `rr:child`/`rr:parent` order.
    child: Vec<usize>,
    /// Shared so cloning a [`CompiledMap`] never re-scans or re-allocates the index.
    index: std::sync::Arc<HashMap<Vec<String>, Vec<GenTerm>>>,
}

/// An object of a predicate-object map: an ordinary term map, or a join into a parent table.
#[derive(Clone, Debug)]
enum ObjectSpec {
    Term(TermSpec),
    Ref(RefJoin),
}

/// One `rr:predicateObjectMap`: every predicate × every object (the R2RML cartesian rule).
#[derive(Clone, Debug)]
struct CompiledPom {
    predicates: Vec<TermSpec>,
    objects: Vec<ObjectSpec>,
    /// `rr:graphMap`/`rr:graph` on THIS map — the union with the subject map's (R2RML §12.2).
    graphs: Vec<TermSpec>,
}

/// A fully header-resolved mapping for ONE logical table (one CSV file).
#[derive(Clone, Debug)]
struct CompiledMap {
    subject: TermSpec,
    /// The `rr:class` IRIs, pre-serialised as N-Triples object terms (`<iri>`).
    classes: Vec<String>,
    poms: Vec<CompiledPom>,
    /// `rr:graphMap`/`rr:graph` on the SUBJECT map: scopes the class + provenance triples and
    /// unions into every predicate-object map's graphs.
    graphs: Vec<TermSpec>,
    /// `--row-provenance`: the row-IRI prefix (`<base><table>/row/`), completed with the
    /// 1-based data-row number.
    provenance: Option<String>,
    /// Resolve relative generated IRIs against this (simple prefix concatenation).
    base: Option<String>,
    /// Expected record width (= header width); a ragged row is a loud error.
    width: usize,
}

impl CompiledMap {
    /// Does this map use ANY graph map? Drives N-Triples vs N-Quads emission: a graph-map-free
    /// mapping keeps the unchanged streaming N-Triples path. Deliberately CONSERVATIVE — a
    /// mapping whose graph maps all resolve to `rr:defaultGraph` still takes the quad path,
    /// because that is only knowable per row, and mis-taking the triple path would lose data.
    fn uses_graphs(&self) -> bool {
        !self.graphs.is_empty() || self.poms.iter().any(|p| !p.graphs.is_empty())
    }
}

/// Resolve a column reference against the header: a `"quoted"` (delimited) name matches
/// exactly after unquoting; a bare name matches exactly first, then case-insensitively
/// (SQL unquoted-identifier semantics) — ambiguity is an error.
fn resolve_col(name: &str, header: &[String]) -> Result<usize, String> {
    let delimited = name.len() >= 2 && name.starts_with('"') && name.ends_with('"');
    let bare = if delimited { &name[1..name.len() - 1] } else { name };
    if let Some(i) = header.iter().position(|h| h == bare) {
        return Ok(i);
    }
    if !delimited {
        let ci: Vec<usize> =
            (0..header.len()).filter(|&i| header[i].eq_ignore_ascii_case(bare)).collect();
        match ci.len() {
            1 => return Ok(ci[0]),
            n if n > 1 => return Err(format!("column reference {name:?} is ambiguous (case-insensitive) in header {header:?}")),
            _ => {}
        }
    }
    Err(format!("column {name:?} not found in CSV header {header:?}"))
}

/// Parse an R2RML-style string template: `{column}` references, `\{` `\}` `\\` escapes.
/// `{_row}` (the 1-based data-row number) is only legal where `allow_row` is set (the
/// direct-mapping subject template) — R2RML templates stay spec-clean.
fn parse_template(t: &str, header: &[String], allow_row: bool) -> Result<Vec<TplPart>, String> {
    let mut parts: Vec<TplPart> = Vec::new();
    let mut text = String::new();
    let mut chars = t.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(e @ ('{' | '}' | '\\')) => text.push(e),
                Some(other) => return Err(format!("invalid escape '\\{other}' in template {t:?}")),
                None => return Err(format!("dangling '\\' at end of template {t:?}")),
            },
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(ch) => name.push(ch),
                        None => return Err(format!("unclosed '{{' in template {t:?}")),
                    }
                }
                if !text.is_empty() {
                    parts.push(TplPart::Text(std::mem::take(&mut text)));
                }
                if name == "_row" {
                    if !allow_row {
                        return Err("{_row} is a direct-mapping placeholder; it is not available in R2RML templates".into());
                    }
                    parts.push(TplPart::Row);
                } else {
                    parts.push(TplPart::Col(resolve_col(&name, header)?));
                }
            }
            '}' => return Err(format!("unbalanced '}}' in template {t:?}")),
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        parts.push(TplPart::Text(text));
    }
    if parts.is_empty() {
        return Err("empty template".into());
    }
    Ok(parts)
}

/// Generate the term a spec produces for one row. `Ok(None)` = NULL (an empty referenced
/// cell): the triple — or the whole row, for a subject — is skipped, per R2RML.
fn gen_term(
    spec: &TermSpec,
    row: &[String],
    row_num: u64,
    base: Option<&str>,
) -> Result<Option<GenTerm>, String> {
    let raw = match &spec.value {
        ValueSpec::Constant(t) => return Ok(Some(t.clone())),
        ValueSpec::Column(i) => {
            let v = &row[*i];
            if v.is_empty() {
                return Ok(None);
            }
            v.clone()
        }
        ValueSpec::Template(parts) => {
            // IRI term maps percent-encode each SUBSTITUTED value (never the fixed text);
            // literal/blank-node maps substitute raw values.
            let encode = spec.kind == TermKind::Iri;
            let mut out = String::new();
            for p in parts {
                match p {
                    TplPart::Text(t) => out.push_str(t),
                    TplPart::Row => out.push_str(&row_num.to_string()),
                    TplPart::Col(i) => {
                        let v = &row[*i];
                        if v.is_empty() {
                            return Ok(None);
                        }
                        if encode {
                            out.push_str(&iri_safe(v));
                        } else {
                            out.push_str(v);
                        }
                    }
                }
            }
            out
        }
    };
    Ok(Some(match &spec.kind {
        TermKind::Iri => {
            let iri = if raw.contains(':') || base.is_none() {
                raw
            } else {
                format!("{}{raw}", base.unwrap_or_default())
            };
            check_iri(&iri)?;
            GenTerm::Iri(iri)
        }
        TermKind::Blank => GenTerm::Blank(blank_label(&raw)),
        TermKind::LitPlain => GenTerm::Lit { value: raw, datatype: None, lang: None },
        TermKind::LitTyped(dt) => GenTerm::Lit { value: raw, datatype: Some(dt.clone()), lang: None },
        TermKind::LitLang(l) => GenTerm::Lit { value: raw, datatype: None, lang: Some(l.clone()) },
        TermKind::LitInfer => {
            let dt = infer_datatype(&raw).map(str::to_owned);
            GenTerm::Lit { value: raw, datatype: dt, lang: None }
        }
    }))
}

/// What one scope's graph maps generated for one row.
struct GraphSet {
    /// The generated graph labels, N-Quads-serialised and duplicate-free; the EMPTY string is
    /// the default graph (an `rr:defaultGraph` constant).
    labels: Vec<String>,
    /// A declared graph map generated NULL on this row (and so contributed no label). This
    /// distinguishes "no graph maps declared" — the default graph — from "declared, but every
    /// one was NULL", which must NOT silently fall back to the default graph.
    null_seen: bool,
}

/// The graph terms a set of graph maps generates for one row. A NULL graph map contributes
/// NOTHING to the set rather than erasing it: R2RML §12.2 scopes a statement by the UNION of
/// its subject map's and its predicate-object map's graph sets, so a NULL must not wipe out a
/// graph a sibling map — or the other side of that union — generated.
fn gen_graphs(
    specs: &[TermSpec],
    row: &[String],
    row_num: u64,
    base: Option<&str>,
) -> Result<GraphSet, String> {
    let mut labels: Vec<String> = Vec::with_capacity(specs.len());
    let mut null_seen = false;
    for spec in specs {
        let Some(t) = gen_term(spec, row, row_num, base)? else {
            null_seen = true;
            continue;
        };
        let label = match t {
            GenTerm::Iri(i) if i == RR_DEFAULT_GRAPH => String::new(),
            GenTerm::Iri(i) => format!("<{i}>"),
            other => {
                return Err(format!("graph map generated a non-IRI term {other:?} on row {row_num}"))
            }
        };
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    Ok(GraphSet { labels, null_seen })
}

/// The union of a subject map's graphs and a predicate-object map's graphs (R2RML §12.2),
/// duplicate-free and order-preserving. Both empty → an empty set → the default graph.
fn union_graphs(subject: &[String], pom: &[String]) -> Vec<String> {
    if pom.is_empty() {
        return subject.to_vec();
    }
    let mut out: Vec<String> = Vec::with_capacity(subject.len() + pom.len());
    for g in subject.iter().chain(pom) {
        if !out.contains(g) {
            out.push(g.clone());
        }
    }
    out
}

/// Write one statement once per graph in `graphs` — an EMPTY slice (or an empty label) is the
/// default graph, which serialises as a plain N-Triples line.
fn push_stmt(out: &mut String, s: &str, p: &str, o: &str, graphs: &[String]) {
    let mut one = |g: &str| {
        out.push_str(s);
        out.push(' ');
        out.push_str(p);
        out.push(' ');
        out.push_str(o);
        if !g.is_empty() {
            out.push(' ');
            out.push_str(g);
        }
        out.push_str(" .\n");
    };
    if graphs.is_empty() {
        one("");
    } else {
        for g in graphs {
            one(g);
        }
    }
}

/// Emit the N-Triples (or, with graph maps, N-Quads) lines one CSV row generates under a
/// compiled map.
fn emit_row(map: &CompiledMap, row: &[String], row_num: u64, out: &mut String) -> Result<(), String> {
    if row.len() != map.width {
        return Err(format!(
            "ragged CSV row {row_num}: {} field(s), header has {}",
            row.len(),
            map.width
        ));
    }
    let base = map.base.as_deref();
    let Some(subj) = gen_term(&map.subject, row, row_num, base)? else {
        return Ok(()); // NULL subject: the whole row generates nothing.
    };
    if matches!(subj, GenTerm::Lit { .. }) {
        return Err(format!("subject map generated a literal on row {row_num}"));
    }
    let s_nt = nt_term(&subj);
    let s_graphs = gen_graphs(&map.graphs, row, row_num, base)?;
    // Class and provenance triples are scoped by the subject map's graph set alone. An empty
    // set means the default graph — UNLESS graph maps were declared and every one generated
    // NULL, in which case the statement is unscoped and dropped (the same fail-quiet-per-NULL
    // rule R2RML gives a NULL object, never a silent default-graph fallback that would put data
    // somewhere the mapping did not ask for).
    if !(s_graphs.labels.is_empty() && s_graphs.null_seen) {
        for cls in &map.classes {
            push_stmt(out, &s_nt, RDF_TYPE_NT, cls, &s_graphs.labels);
        }
        if let Some(prefix) = &map.provenance {
            push_stmt(
                out,
                &s_nt,
                &format!("<{PROV_WAS_DERIVED_FROM}>"),
                &format!("<{prefix}{row_num}>"),
                &s_graphs.labels,
            );
        }
    }
    for pom in &map.poms {
        let p_graphs = gen_graphs(&pom.graphs, row, row_num, base)?;
        let graphs = union_graphs(&s_graphs.labels, &p_graphs.labels);
        if graphs.is_empty() && (s_graphs.null_seen || p_graphs.null_seen) {
            continue; // every declared graph map was NULL: this map's statements are unscoped.
        }
        let mut preds: Vec<String> = Vec::with_capacity(pom.predicates.len());
        for p in &pom.predicates {
            match gen_term(p, row, row_num, base)? {
                None => {}
                Some(GenTerm::Iri(i)) => preds.push(format!("<{i}>")),
                Some(other) => {
                    return Err(format!("predicate map generated a non-IRI term {other:?} on row {row_num}"))
                }
            }
        }
        if preds.is_empty() {
            continue;
        }
        for o in &pom.objects {
            match o {
                ObjectSpec::Term(spec) => {
                    let Some(obj) = gen_term(spec, row, row_num, base)? else {
                        continue; // NULL object: skip this triple only.
                    };
                    let o_nt = nt_term(&obj);
                    for p_nt in &preds {
                        push_stmt(out, &s_nt, p_nt, &o_nt, &graphs);
                    }
                }
                ObjectSpec::Ref(join) => {
                    // SQL join semantics: a NULL (empty) child key cell matches no parent row.
                    let mut key: Vec<String> = Vec::with_capacity(join.child.len());
                    for &i in &join.child {
                        if row[i].is_empty() {
                            break;
                        }
                        key.push(row[i].clone());
                    }
                    if key.len() != join.child.len() {
                        continue;
                    }
                    let Some(parents) = join.index.get(&key) else { continue };
                    for parent in parents {
                        let o_nt = nt_term(parent);
                        for p_nt in &preds {
                            push_stmt(out, &s_nt, p_nt, &o_nt, &graphs);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Streaming: CSV rows → N-Triples chunks → `Read`
// ---------------------------------------------------------------------------------------------

/// A stream of already-parsed CSV records — the one row-source type the compiler and the
/// emitter share, so a table can be pre-scanned (join index) and streamed (emission) alike.
type RowIter = Box<dyn Iterator<Item = Result<Vec<String>, String>> + Send>;

/// Iterator adapter: each CSV data row becomes one N-Triples/N-Quads chunk (possibly empty).
struct MappedRows {
    rows: RowIter,
    map: CompiledMap,
    row_num: u64,
    failed: bool,
}

impl Iterator for MappedRows {
    type Item = Result<String, String>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.rows.next()? {
            Err(e) => {
                self.failed = true;
                Some(Err(e))
            }
            Ok(row) => {
                self.row_num += 1;
                let mut chunk = String::new();
                match emit_row(&self.map, &row, self.row_num, &mut chunk) {
                    Ok(()) => Some(Ok(chunk)),
                    Err(e) => {
                        self.failed = true;
                        Some(Err(e))
                    }
                }
            }
        }
    }
}

/// A chunk iterator whose items feed a `Read` (what `Graph::load_reader_parallel` and the
/// `--out` copy loop consume). Multiple sources (one per triples map / CSV file) run in
/// sequence. A mapping error surfaces as an `io::Error`, so the ingest fails loudly.
struct NtReader {
    srcs: Vec<Box<dyn Iterator<Item = Result<String, String>> + Send>>,
    idx: usize,
    buf: Vec<u8>,
    pos: usize,
}

impl NtReader {
    fn new(srcs: Vec<Box<dyn Iterator<Item = Result<String, String>> + Send>>) -> Self {
        NtReader { srcs, idx: 0, buf: Vec::new(), pos: 0 }
    }
}

impl Read for NtReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.pos < self.buf.len() {
                let n = (self.buf.len() - self.pos).min(out.len());
                out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            let Some(src) = self.srcs.get_mut(self.idx) else { return Ok(0) };
            match src.next() {
                None => self.idx += 1,
                Some(Ok(chunk)) => {
                    self.buf = chunk.into_bytes();
                    self.pos = 0;
                }
                Some(Err(e)) => return Err(std::io::Error::other(e)),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Direct mapping
// ---------------------------------------------------------------------------------------------

/// Direct-mapping knobs (all flag-fed; defaults documented on the module).
struct DirectOpts {
    base: String,
    template: Option<String>,
    /// `None` = default class `<base><table>`; `Some(None)` = `--class none`; `Some(Some(iri))`.
    class: Option<Option<String>>,
    infer: bool,
    /// `--row-provenance`: also emit `prov:wasDerivedFrom <base><table>/row/{_row}>`.
    provenance: bool,
}

/// Compile the direct mapping of one CSV header.
fn compile_direct(header: &[String], table: &str, opts: &DirectOpts) -> Result<CompiledMap, String> {
    check_header(header)?;
    let t_enc = iri_safe(table);
    let tpl = match &opts.template {
        Some(t) => t.clone(),
        None => format!("{}{t_enc}/row/{{_row}}", opts.base),
    };
    let subject = TermSpec { value: ValueSpec::Template(parse_template(&tpl, header, true)?), kind: TermKind::Iri };
    let classes: Vec<String> = match &opts.class {
        None => vec![format!("{}{t_enc}", opts.base)],
        Some(None) => Vec::new(),
        Some(Some(iri)) => vec![iri.clone()],
    };
    for c in &classes {
        check_iri(c)?;
    }
    let classes: Vec<String> = classes.iter().map(|c| format!("<{c}>")).collect();
    let poms = header
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let pred = format!("{}{t_enc}#{}", opts.base, iri_safe(col));
            check_iri(&pred)?;
            Ok(CompiledPom {
                predicates: vec![TermSpec { value: ValueSpec::Constant(GenTerm::Iri(pred)), kind: TermKind::Iri }],
                objects: vec![ObjectSpec::Term(TermSpec {
                    value: ValueSpec::Column(i),
                    kind: if opts.infer { TermKind::LitInfer } else { TermKind::LitPlain },
                })],
                graphs: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let provenance = opts.provenance.then(|| row_iri_prefix(&opts.base, table)).transpose()?;
    Ok(CompiledMap {
        subject,
        classes,
        poms,
        graphs: Vec::new(),
        provenance,
        base: Some(opts.base.clone()),
        width: header.len(),
    })
}

/// The `--row-provenance` row-IRI prefix for one table: `<base><table>/row/`, validated once
/// at compile time (against row 1) so a bad `--base` fails before any row is emitted.
fn row_iri_prefix(base: &str, table: &str) -> Result<String, String> {
    let prefix = format!("{base}{}/row/", iri_safe(table));
    check_iri(&format!("{prefix}1"))?;
    Ok(prefix)
}

/// Header sanity: no empty and no duplicate column names (both would map ambiguously).
fn check_header(header: &[String]) -> Result<(), String> {
    for (i, h) in header.iter().enumerate() {
        if h.is_empty() {
            return Err(format!("CSV header column {} is empty", i + 1));
        }
    }
    for i in 0..header.len() {
        if header[i + 1..].contains(&header[i]) {
            return Err(format!("duplicate CSV header column {:?}", header[i]));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// R2RML (materializing subset over CSV logical tables)
// ---------------------------------------------------------------------------------------------

/// The mapping document as a subject→(predicate, object) adjacency over full IRIs — small
/// (a mapping file), so materialising it wholesale via one engine query is fine.
struct RrGraph {
    adj: HashMap<String, Vec<(String, oxrdf::Term)>>,
}

/// The `rr:` local names this subset implements. Any OTHER `rr:` predicate in the mapping is
/// a loud error (fail-closed): `rr:sqlQuery`, `rr:sqlVersion`, `rr:inverseExpression`, … are
/// all rejected, never silently skipped.
const RR_SUPPORTED: &[&str] = &[
    "logicalTable",
    "tableName",
    "subjectMap",
    "subject",
    "class",
    "termType",
    "template",
    "column",
    "constant",
    "predicateObjectMap",
    "predicateMap",
    "predicate",
    "objectMap",
    "object",
    "datatype",
    "language",
    // [OPUS-5] (sq-u1z86) cross-CSV joins + named-graph output.
    "parentTriplesMap",
    "joinCondition",
    "child",
    "parent",
    "graphMap",
    "graph",
];

impl RrGraph {
    /// Values of `pred` on `node` (node key = the term's N-Triples-ish display form).
    fn props<'a>(&'a self, node: &str, pred: &str) -> impl Iterator<Item = &'a oxrdf::Term> {
        let full = format!("{RR}{pred}");
        self.adj.get(node).into_iter().flatten().filter_map(move |(p, o)| (p == &full).then_some(o))
    }

    /// At most one value of `pred` on `node` (two is a mapping error).
    fn prop1(&self, node: &str, pred: &str) -> Result<Option<&oxrdf::Term>, String> {
        let mut it = self.props(node, pred);
        let first = it.next();
        if it.next().is_some() {
            return Err(format!("term map {node} has more than one rr:{pred}"));
        }
        Ok(first)
    }
}

/// A parsed R2RML mapping: the adjacency + each triples map's node key and CSV table name.
struct R2rmlMapping {
    rr: RrGraph,
    /// (triples-map node key, logical table name — surrounding `"` stripped).
    tms: Vec<(String, String)>,
}

fn term_key(t: &oxrdf::Term) -> String {
    t.to_string()
}

fn lit_value(t: &oxrdf::Term, what: &str) -> Result<String, String> {
    match t {
        oxrdf::Term::Literal(l) => Ok(l.value().to_owned()),
        other => Err(format!("{what} must be a literal, got {other}")),
    }
}

fn iri_value(t: &oxrdf::Term, what: &str) -> Result<String, String> {
    match t {
        oxrdf::Term::NamedNode(n) => Ok(n.as_str().to_owned()),
        other => Err(format!("{what} must be an IRI, got {other}")),
    }
}

/// Parse an R2RML mapping document (Turtle) into [`R2rmlMapping`], failing loudly on any
/// `rr:` construct outside the supported materializing-CSV subset.
fn parse_r2rml(mapping_ttl: &str) -> Result<R2rmlMapping, String> {
    // Resolve relative IRIs (the conventional `<#TriplesMap1>` style) against the same base
    // the W3C R2RML test suite uses; map-node identity is internal, so the exact base is
    // immaterial to the generated triples.
    let g = sparq_core::Graph::load_str_with_base(mapping_ttl, "turtle", "http://example.com/base/")
        .map_err(|e| format!("parsing R2RML mapping: {e}"))?;
    let res = sparq_engine::query(&g, "SELECT ?s ?p ?o WHERE { ?s ?p ?o }")
        .map_err(|e| format!("reading R2RML mapping graph: {e}"))?;
    let mut adj: HashMap<String, Vec<(String, oxrdf::Term)>> = HashMap::new();
    for row in &res.rows {
        let (Some(Some(s)), Some(Some(p)), Some(Some(o))) = (row.first(), row.get(1), row.get(2)) else {
            return Err("unexpected unbound value in mapping graph".into());
        };
        let p_iri = iri_value(p, "predicate")?;
        if let Some(local) = p_iri.strip_prefix(RR) {
            if !RR_SUPPORTED.contains(&local) {
                return Err(format!(
                    "unsupported R2RML construct rr:{local} — this materializing subset covers CSV logical tables only \
                     (no rr:sqlQuery / rr:sqlVersion / rr:inverseExpression: SQL-connection R2RML is out of scope)"
                ));
            }
        }
        adj.entry(term_key(s)).or_default().push((p_iri, o.clone()));
    }
    let rr = RrGraph { adj };
    // A triples map is any resource with an rr:logicalTable (R2RML's structural definition).
    let mut tms: Vec<(String, String)> = Vec::new();
    let mut keys: Vec<&String> = rr.adj.keys().collect();
    keys.sort(); // deterministic emission order
    for node in keys {
        let Some(lt) = rr.prop1(node, "logicalTable")? else { continue };
        let lt_key = term_key(lt);
        let name = rr
            .prop1(&lt_key, "tableName")?
            .ok_or_else(|| format!("logical table of {node} has no rr:tableName (CSV logical tables only)"))?;
        let name = lit_value(name, "rr:tableName")?;
        let name = name.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(&name).to_owned();
        tms.push((node.clone(), name));
    }
    Ok(R2rmlMapping { rr, tms })
}

/// Build the [`TermSpec`] of one term-map node. `pos`: 0 = subject, 1 = predicate, 2 = object,
/// 3 = graph (drives the R2RML default term type + which properties are legal).
fn term_spec(
    rr: &RrGraph,
    node: &str,
    pos: u8,
    header: &[String],
) -> Result<TermSpec, String> {
    let template = rr.prop1(node, "template")?.map(|t| lit_value(t, "rr:template")).transpose()?;
    let column = rr.prop1(node, "column")?.map(|t| lit_value(t, "rr:column")).transpose()?;
    let constant = rr.prop1(node, "constant")?.cloned();
    let n_kinds = [template.is_some(), column.is_some(), constant.is_some()].iter().filter(|b| **b).count();
    if n_kinds != 1 {
        return Err(format!("term map {node} must have exactly one of rr:template / rr:column / rr:constant (found {n_kinds})"));
    }
    let datatype = rr.prop1(node, "datatype")?.map(|t| iri_value(t, "rr:datatype")).transpose()?;
    let language = rr.prop1(node, "language")?.map(|t| lit_value(t, "rr:language")).transpose()?;
    if datatype.is_some() && language.is_some() {
        return Err(format!("term map {node} has both rr:datatype and rr:language"));
    }
    let term_type = match rr.prop1(node, "termType")? {
        None => {
            // R2RML §7.4.1 defaults: object position is a Literal iff the map is
            // column-valued or carries rr:datatype/rr:language; everything else is an IRI.
            if pos == 2 && (column.is_some() || datatype.is_some() || language.is_some()) {
                "Literal".to_owned()
            } else {
                "IRI".to_owned()
            }
        }
        Some(t) => {
            let t = iri_value(t, "rr:termType")?;
            t.strip_prefix(RR).ok_or_else(|| format!("invalid rr:termType {t}"))?.to_owned()
        }
    };
    if (datatype.is_some() || language.is_some()) && term_type != "Literal" {
        return Err(format!("term map {node}: rr:datatype/rr:language require rr:termType rr:Literal"));
    }
    let kind = match term_type.as_str() {
        "IRI" => TermKind::Iri,
        "BlankNode" => {
            if pos == 1 || pos == 3 {
                // R2RML: predicate maps and graph maps must generate IRIs.
                return Err(format!(
                    "{} map {node} cannot be a blank node",
                    if pos == 1 { "predicate" } else { "graph" }
                ));
            }
            TermKind::Blank
        }
        "Literal" => {
            if pos != 2 {
                return Err(format!("term map {node}: rr:Literal is only valid in object position"));
            }
            if let Some(l) = &language {
                TermKind::LitLang(l.clone())
            } else if let Some(dt) = &datatype {
                if dt == XSD_STRING {
                    TermKind::LitPlain
                } else {
                    TermKind::LitTyped(dt.clone())
                }
            } else {
                TermKind::LitPlain
            }
        }
        other => return Err(format!("unsupported rr:termType rr:{other}")),
    };
    let value = if let Some(t) = template {
        ValueSpec::Template(parse_template(&t, header, false)?)
    } else if let Some(c) = column {
        ValueSpec::Column(resolve_col(&c, header)?)
    } else {
        ValueSpec::Constant(constant_term(&constant.expect("checked above"), pos)?)
    };
    Ok(TermSpec { value, kind })
}

/// Convert an `rr:constant` (or `rr:subject`/`rr:predicate`/`rr:object` shortcut) value.
fn constant_term(t: &oxrdf::Term, pos: u8) -> Result<GenTerm, String> {
    match t {
        oxrdf::Term::NamedNode(n) => Ok(GenTerm::Iri(n.as_str().to_owned())),
        oxrdf::Term::Literal(l) => {
            if pos != 2 {
                return Err(format!("constant literal {l} is only valid in object position"));
            }
            let lang = l.language().map(str::to_owned);
            let dt = if lang.is_some() || l.datatype().as_str() == XSD_STRING {
                None
            } else {
                Some(l.datatype().as_str().to_owned())
            };
            Ok(GenTerm::Lit { value: l.value().to_owned(), datatype: dt, lang })
        }
        other => Err(format!("unsupported constant term {other} (blank-node constants are not shared-scope-safe)")),
    }
}

/// The subject side of a triples map — `rr:subjectMap` node or the `rr:subject` constant
/// shortcut (exactly one) — plus its `rr:class`es and its `rr:graphMap`/`rr:graph` set.
/// Shared by [`compile_tm`] and the join pre-scan, which needs the PARENT's subject map.
fn subject_of(
    m: &R2rmlMapping,
    tm_key: &str,
    header: &[String],
) -> Result<(TermSpec, Vec<String>, Vec<TermSpec>), String> {
    let rr = &m.rr;
    let sm = rr.prop1(tm_key, "subjectMap")?;
    let s_const = rr.prop1(tm_key, "subject")?;
    match (sm, s_const) {
        (Some(sm), None) => {
            let sm_key = term_key(sm);
            let spec = term_spec(rr, &sm_key, 0, header)?;
            let mut classes = Vec::new();
            for c in rr.props(&sm_key, "class") {
                classes.push(format!("<{}>", iri_value(c, "rr:class")?));
            }
            classes.sort();
            Ok((spec, classes, graph_specs(rr, &sm_key, header)?))
        }
        (None, Some(c)) => Ok((
            TermSpec { value: ValueSpec::Constant(constant_term(c, 0)?), kind: TermKind::Iri },
            Vec::new(),
            Vec::new(),
        )),
        (None, None) => Err(format!("triples map {tm_key} has no rr:subjectMap / rr:subject")),
        (Some(_), Some(_)) => Err(format!("triples map {tm_key} has both rr:subjectMap and rr:subject")),
    }
}

/// The `rr:graph` constants + `rr:graphMap` term maps attached to one subject / predicate-object
/// map node. [OPUS-5] (sq-u1z86)
fn graph_specs(rr: &RrGraph, node: &str, header: &[String]) -> Result<Vec<TermSpec>, String> {
    let mut out: Vec<TermSpec> = Vec::new();
    for g in rr.props(node, "graph") {
        out.push(TermSpec { value: ValueSpec::Constant(constant_term(g, 3)?), kind: TermKind::Iri });
    }
    let mut keys: Vec<String> = rr.props(node, "graphMap").map(term_key).collect();
    keys.sort(); // deterministic emission order
    for key in keys {
        let spec = term_spec(rr, &key, 3, header)?;
        if spec.kind != TermKind::Iri {
            return Err(format!("graph map {key} must generate an IRI"));
        }
        out.push(spec);
    }
    Ok(out)
}

/// Opens a CSV logical table by name, returning its header + a fresh row stream. Threading it
/// through the compiler is what lets a referencing object map pre-scan its PARENT table.
type TableOpener<'a> = &'a mut dyn FnMut(&str) -> Result<(Vec<String>, RowIter), String>;

/// Compile one triples map against its CSV header. `provenance` is the `--row-provenance`
/// row-IRI prefix for THIS table (`None` = off); `open` resolves parent tables for joins.
fn compile_tm(
    m: &R2rmlMapping,
    tm_key: &str,
    header: &[String],
    base: Option<&str>,
    provenance: Option<&str>,
    open: TableOpener,
) -> Result<CompiledMap, String> {
    check_header(header)?;
    let rr = &m.rr;
    let (subject, classes, graphs) = subject_of(m, tm_key, header)?;
    let mut poms: Vec<CompiledPom> = Vec::new();
    let mut pom_keys: Vec<String> = rr.props(tm_key, "predicateObjectMap").map(term_key).collect();
    pom_keys.sort(); // deterministic emission order
    for pom_key in pom_keys {
        let mut predicates: Vec<TermSpec> = Vec::new();
        for p in rr.props(&pom_key, "predicate") {
            predicates.push(TermSpec { value: ValueSpec::Constant(constant_term(p, 1)?), kind: TermKind::Iri });
        }
        for pm in rr.props(&pom_key, "predicateMap") {
            predicates.push(term_spec(rr, &term_key(pm), 1, header)?);
        }
        let mut objects: Vec<ObjectSpec> = Vec::new();
        for o in rr.props(&pom_key, "object") {
            let c = constant_term(o, 2)?;
            let kind = match &c {
                GenTerm::Iri(_) => TermKind::Iri,
                _ => TermKind::LitPlain, // kind is unused for constants; value wins
            };
            objects.push(ObjectSpec::Term(TermSpec { value: ValueSpec::Constant(c), kind }));
        }
        let mut om_keys: Vec<String> = rr.props(&pom_key, "objectMap").map(term_key).collect();
        om_keys.sort(); // deterministic emission order
        for om_key in om_keys {
            // A REFERENCING object map (rr:parentTriplesMap) joins into another table; every
            // other object map is an ordinary term map over this row.
            objects.push(match rr.prop1(&om_key, "parentTriplesMap")?.cloned() {
                Some(parent) => ObjectSpec::Ref(compile_ref(m, &om_key, &parent, header, base, open)?),
                None => ObjectSpec::Term(term_spec(rr, &om_key, 2, header)?),
            });
        }
        if predicates.is_empty() || objects.is_empty() {
            return Err(format!("predicate-object map {pom_key} needs at least one predicate and one object"));
        }
        poms.push(CompiledPom { predicates, objects, graphs: graph_specs(rr, &pom_key, header)? });
    }
    Ok(CompiledMap {
        subject,
        classes,
        poms,
        graphs,
        provenance: provenance.map(str::to_owned),
        base: base.map(str::to_owned),
        width: header.len(),
    })
}

/// [OPUS-5] (sq-u1z86) Compile a referencing object map into a [`RefJoin`]: resolve the
/// `rr:joinCondition`s, then PRE-SCAN the parent CSV once into the `key tuple → parent
/// subjects` hash index the child rows probe as they stream.
fn compile_ref(
    m: &R2rmlMapping,
    om_key: &str,
    parent: &oxrdf::Term,
    child_header: &[String],
    base: Option<&str>,
    open: TableOpener,
) -> Result<RefJoin, String> {
    let rr = &m.rr;
    for p in ["template", "column", "constant", "datatype", "language", "termType"] {
        if rr.prop1(om_key, p)?.is_some() {
            return Err(format!("referencing object map {om_key} must not also have rr:{p}"));
        }
    }
    let parent_key = term_key(parent);
    let parent_table = m
        .tms
        .iter()
        .find(|(k, _)| k == &parent_key)
        .map(|(_, t)| t.clone())
        .ok_or_else(|| format!("rr:parentTriplesMap {parent_key} is not a triples map with an rr:logicalTable"))?;
    let mut jc_keys: Vec<String> = rr.props(om_key, "joinCondition").map(term_key).collect();
    jc_keys.sort(); // key-tuple order must be stable across child and parent
    if jc_keys.is_empty() {
        return Err(format!(
            "referencing object map {om_key} has no rr:joinCondition — this materializing subset needs an \
             explicit rr:child/rr:parent join (R2RML's condition-free form is a SQL cross join, out of scope)"
        ));
    }
    let mut child: Vec<usize> = Vec::with_capacity(jc_keys.len());
    let mut parent_cols: Vec<String> = Vec::with_capacity(jc_keys.len());
    for jc in &jc_keys {
        let c = rr.prop1(jc, "child")?.ok_or_else(|| format!("join condition {jc} has no rr:child"))?;
        let p = rr.prop1(jc, "parent")?.ok_or_else(|| format!("join condition {jc} has no rr:parent"))?;
        child.push(resolve_col(&lit_value(c, "rr:child")?, child_header)?);
        parent_cols.push(lit_value(p, "rr:parent")?);
    }
    let (parent_header, rows) = open(&parent_table)
        .map_err(|e| format!("parent logical table {parent_table:?} of {om_key}: {e}"))?;
    check_header(&parent_header)?;
    let (subject, _, _) = subject_of(m, &parent_key, &parent_header)?;
    let parent_idx: Vec<usize> =
        parent_cols.iter().map(|c| resolve_col(c, &parent_header)).collect::<Result<_, String>>()?;
    let mut index: HashMap<Vec<String>, Vec<GenTerm>> = HashMap::new();
    let mut row_num: u64 = 0;
    for row in rows {
        let row = row.map_err(|e| format!("parent table {parent_table:?}: {e}"))?;
        row_num += 1;
        if row.len() != parent_header.len() {
            return Err(format!(
                "ragged CSV row {row_num} in parent table {parent_table:?}: {} field(s), header has {}",
                row.len(),
                parent_header.len()
            ));
        }
        // SQL join semantics: a NULL (empty) key cell joins to nothing, so it never enters
        // the index — and neither does a row whose subject map is itself NULL.
        let mut key: Vec<String> = Vec::with_capacity(parent_idx.len());
        for &i in &parent_idx {
            if row[i].is_empty() {
                break;
            }
            key.push(row[i].clone());
        }
        if key.len() != parent_idx.len() {
            continue;
        }
        let Some(s) = gen_term(&subject, &row, row_num, base)? else { continue };
        if matches!(s, GenTerm::Lit { .. }) {
            return Err(format!("parent subject map generated a literal on row {row_num} of {parent_table:?}"));
        }
        let bucket = index.entry(key).or_default();
        // The same key can repeat; the RDF output is a SET, so identical subjects collapse.
        if !bucket.contains(&s) {
            bucket.push(s);
        }
    }
    Ok(RefJoin { child, index: std::sync::Arc::new(index) })
}

// ---------------------------------------------------------------------------------------------
// CLI driver
// ---------------------------------------------------------------------------------------------

/// The file stem used as the logical table name: file name minus directory, minus one
/// compression extension (`.gz`/`.zst`/`.zstd`/`.bz2`), minus one format extension.
fn table_stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let name = ["gz", "zst", "zstd", "bz2"]
        .iter()
        .find_map(|e| name.strip_suffix(&format!(".{e}")))
        .unwrap_or(name);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

fn usage() -> ! {
    eprintln!(
        "usage: sparq-cli tabular <csv[.gz|.zst|.bz2]> [<name>=<csv> ...] [flags]\n\
         \n  direct mapping (default):\
         \n    --base <iri>        vocabulary/subject base (default http://example.com/)\
         \n    --template <tpl>    subject IRI template; {{col}} + {{_row}} placeholders\
         \n                        (default <base><table>/row/{{_row}})\
         \n    --class <iri|none>  per-row rdf:type (default <base><table>)\
         \n    --no-infer          disable datatype inference (all cells plain strings)\
         \n  R2RML:\
         \n    --mapping <r2rml.ttl>  execute the mapping (CSV logical tables bound by\
         \n                        rr:tableName = file stem, or explicit <name>=<csv>)\
         \n                        joins (rr:parentTriplesMap + rr:joinCondition) and named\
         \n                        graphs (rr:graphMap/rr:graph -> N-Quads) are supported\
         \n  common:\
         \n    --row-provenance    also emit <subject> prov:wasDerivedFrom\
         \n                        <base><table>/row/{{_row}}\
         \n    --sep <char|tab>    field separator (default ',')\
         \n    --out <file.nt|.nq[.gz|.zst]>  stream the triples/quads out instead of loading\
         \n    --query <sparql> [--format <table|tsv|csv|xml|json|ntriples>] [--count]\
         \n                        run one query over the loaded graph"
    );
    std::process::exit(2);
}

struct Flags {
    files: Vec<(Option<String>, String)>, // (explicit table name, path)
    mapping: Option<String>,
    base: Option<String>,
    template: Option<String>,
    class: Option<String>,
    sep: u8,
    infer: bool,
    row_provenance: bool,
    out: Option<String>,
    query: Option<String>,
}

fn parse_flags(args: &[String]) -> Flags {
    let mut f = Flags {
        files: Vec::new(),
        mapping: None,
        base: None,
        template: None,
        class: None,
        sep: b',',
        infer: true,
        row_provenance: false,
        out: None,
        query: None,
    };
    let mut i = 2;
    let value = |args: &[String], i: usize| -> String {
        args.get(i + 1).cloned().unwrap_or_else(|| {
            eprintln!("{} needs a value", args[i]);
            std::process::exit(2);
        })
    };
    while i < args.len() {
        match args[i].as_str() {
            "--mapping" => {
                f.mapping = Some(value(args, i));
                i += 2;
            }
            "--base" => {
                f.base = Some(value(args, i));
                i += 2;
            }
            "--template" => {
                f.template = Some(value(args, i));
                i += 2;
            }
            "--class" => {
                f.class = Some(value(args, i));
                i += 2;
            }
            "--sep" => {
                let v = value(args, i);
                f.sep = match v.as_str() {
                    "tab" | "\\t" => b'\t',
                    s if s.len() == 1 && s.is_ascii() => s.as_bytes()[0],
                    _ => {
                        eprintln!("--sep must be a single ASCII character (or 'tab')");
                        std::process::exit(2);
                    }
                };
                i += 2;
            }
            "--no-infer" => {
                f.infer = false;
                i += 1;
            }
            // [OPUS-5] (sq-u1z86) Mode-agnostic: row provenance applies to direct mapping
            // AND R2RML, so it is deliberately absent from the direct-mapping-only check.
            "--row-provenance" => {
                f.row_provenance = true;
                i += 1;
            }
            "--out" => {
                f.out = Some(value(args, i));
                i += 2;
            }
            "--query" => {
                f.query = Some(value(args, i));
                i += 2;
            }
            // `--format`/`--count` belong to the query-result emission (read again by
            // `out_format_flag`); just step over them here.
            "--format" => i += 2,
            "--count" => i += 1,
            flag if flag.starts_with("--") => {
                eprintln!("unknown flag {flag}");
                usage();
            }
            positional => {
                match positional.split_once('=') {
                    Some((name, path)) if !name.is_empty() && !path.is_empty() => {
                        f.files.push((Some(name.to_owned()), path.to_owned()));
                    }
                    _ => f.files.push((None, positional.to_owned())),
                }
                i += 1;
            }
        }
    }
    if f.files.is_empty() {
        usage();
    }
    if f.mapping.is_some() && (f.template.is_some() || f.class.is_some() || !f.infer) {
        eprintln!("--template/--class/--no-infer are direct-mapping flags; they do not apply with --mapping");
        std::process::exit(2);
    }
    f
}

/// A CSV row stream over the CLI's transparently-decompressing reader.
type FileRows = CsvRows<Box<dyn Read + Send>>;

/// Open one CSV, read its header, and return (header, the still-streaming row reader).
fn open_csv(path: &str, sep: u8) -> Result<(Vec<String>, FileRows), String> {
    let reader = crate::open_reader(path).map_err(|e| format!("opening {path}: {e}"))?;
    let mut rows = CsvRows::new(reader, sep);
    match rows.next() {
        Some(Ok(header)) => Ok((header, rows)),
        Some(Err(e)) => Err(format!("{path}: {e}")),
        None => Err(format!("{path}: empty CSV (no header row)")),
    }
}

type ChunkIter = Box<dyn Iterator<Item = Result<String, String>> + Send>;

/// Which CSV a logical table name binds to: exact file-stem (or `<name>=<path>`) match first,
/// then a unique case-insensitive one; ambiguity and misses are loud.
fn bind_table(bound: &[(String, String)], table: &str) -> Result<usize, String> {
    let exact: Vec<usize> = (0..bound.len()).filter(|&i| bound[i].0 == table).collect();
    match exact.len() {
        1 => return Ok(exact[0]),
        0 => {}
        _ => return Err(format!("logical table {table:?} matches multiple CSV files")),
    }
    let ci: Vec<usize> = (0..bound.len()).filter(|&i| bound[i].0.eq_ignore_ascii_case(table)).collect();
    match ci.len() {
        1 => Ok(ci[0]),
        0 => Err(format!(
            "no CSV bound for logical table {table:?} (files: {:?}; bind explicitly with {table}=<path>)",
            bound.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
        )),
        _ => Err(format!("logical table {table:?} matches multiple CSV files case-insensitively")),
    }
}

/// Assemble the per-file chunk iterators for the whole invocation, plus whether any compiled
/// map emits into a NAMED graph (`true` → the output is N-Quads and the load is a dataset load).
fn build_sources(f: &Flags) -> Result<(Vec<ChunkIter>, bool), String> {
    let base = f.base.clone().unwrap_or_else(|| "http://example.com/".to_owned());
    let mut srcs: Vec<ChunkIter> = Vec::new();
    let mut quads = false;
    match &f.mapping {
        None => {
            let opts = DirectOpts {
                base: base.clone(),
                template: f.template.clone(),
                class: match f.class.as_deref() {
                    None => None,
                    Some("none") => Some(None),
                    Some(iri) => Some(Some(iri.to_owned())),
                },
                infer: f.infer,
                provenance: f.row_provenance,
            };
            for (name, path) in &f.files {
                let table = name.clone().unwrap_or_else(|| table_stem(path));
                let (header, rows) = open_csv(path, f.sep)?;
                let map = compile_direct(&header, &table, &opts).map_err(|e| format!("{path}: {e}"))?;
                srcs.push(Box::new(MappedRows { rows: Box::new(rows), map, row_num: 0, failed: false }));
            }
        }
        Some(mapping_path) => {
            let mut ttl = String::new();
            crate::open_reader(mapping_path)
                .and_then(|mut r| r.read_to_string(&mut ttl))
                .map_err(|e| format!("reading {mapping_path}: {e}"))?;
            let m = parse_r2rml(&ttl)?;
            // Bind each triples map's table name to a CSV path.
            let bound: Vec<(String, String)> = f
                .files
                .iter()
                .map(|(name, path)| (name.clone().unwrap_or_else(|| table_stem(path)), path.clone()))
                .collect();
            let mut used = vec![false; bound.len()];
            {
                // The opener the compiler also uses to PRE-SCAN a join's parent table — which
                // is why a parent-only CSV counts as "used" and never warns.
                let sep = f.sep;
                let mut open = |table: &str| -> Result<(Vec<String>, RowIter), String> {
                    let idx = bind_table(&bound, table)?;
                    used[idx] = true;
                    let (header, rows) = open_csv(&bound[idx].1, sep)?;
                    Ok((header, Box::new(rows) as RowIter))
                };
                for (tm_key, table) in &m.tms {
                    let path = bound[bind_table(&bound, table)?].1.clone();
                    let (header, rows) = open(table)?;
                    let provenance =
                        f.row_provenance.then(|| row_iri_prefix(&base, table)).transpose()?;
                    let map = compile_tm(&m, tm_key, &header, f.base.as_deref(), provenance.as_deref(), &mut open)
                        .map_err(|e| format!("{path} (table {table:?}): {e}"))?;
                    quads |= map.uses_graphs();
                    srcs.push(Box::new(MappedRows { rows, map, row_num: 0, failed: false }));
                }
            }
            for (i, (table, path)) in bound.iter().enumerate() {
                if !used[i] {
                    eprintln!("warning: {path} (table {table:?}) matched no triples map — it contributes nothing");
                }
            }
        }
    }
    Ok((srcs, quads))
}

/// Sink for `--out`: plain, `.gz`, or `.zst` N-Triples — finished EXPLICITLY so a failed
/// trailer write is an error, not a silently-truncated archive.
enum OutSink {
    Plain(std::io::BufWriter<std::fs::File>),
    Gz(flate2::write::GzEncoder<std::io::BufWriter<std::fs::File>>),
    Zst(Box<zstd::stream::write::Encoder<'static, std::io::BufWriter<std::fs::File>>>),
}

impl OutSink {
    fn create(path: &str) -> std::io::Result<OutSink> {
        let w = std::io::BufWriter::new(std::fs::File::create(path)?);
        Ok(if path.ends_with(".gz") {
            OutSink::Gz(flate2::write::GzEncoder::new(w, flate2::Compression::default()))
        } else if path.ends_with(".zst") || path.ends_with(".zstd") {
            OutSink::Zst(Box::new(zstd::stream::write::Encoder::new(w, 0)?))
        } else {
            OutSink::Plain(w)
        })
    }
    fn writer(&mut self) -> &mut dyn Write {
        match self {
            OutSink::Plain(w) => w,
            OutSink::Gz(w) => w,
            OutSink::Zst(w) => w,
        }
    }
    fn finish(self) -> std::io::Result<()> {
        match self {
            OutSink::Plain(mut w) => w.flush(),
            OutSink::Gz(w) => w.finish().and_then(|mut w| w.flush()),
            OutSink::Zst(w) => w.finish().and_then(|mut w| w.flush()),
        }
    }
}

/// `sparq-cli tabular …` — see [`usage`] and the module docs for the exact contract.
pub(crate) fn cmd_tabular(args: &[String]) {
    let flags = parse_flags(args);
    let (srcs, quads) = build_sources(&flags).unwrap_or_else(|e| die(e));
    let mut reader = NtReader::new(srcs);
    // A mapping with graph maps emits N-Quads; anything else keeps the N-Triples fast path.
    let kind = if quads { "quads" } else { "triples" };
    let t = Instant::now();

    if let Some(out_path) = &flags.out {
        // Stream N-Triples straight out — no graph build, constant memory.
        let mut sink = OutSink::create(out_path).unwrap_or_else(|e| die(format!("creating {out_path}: {e}")));
        let mut buf = vec![0u8; 64 * 1024];
        let mut stmts: u64 = 0;
        loop {
            let n = reader.read(&mut buf).unwrap_or_else(|e| die(e));
            if n == 0 {
                break;
            }
            stmts += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64;
            sink.writer().write_all(&buf[..n]).unwrap_or_else(|e| die(format!("writing {out_path}: {e}")));
        }
        sink.finish().unwrap_or_else(|e| die(format!("finishing {out_path}: {e}")));
        eprintln!("wrote {stmts} {kind} to {out_path} in {:.3}s", t.elapsed().as_secs_f64());
        return;
    }

    let g = if quads {
        // Named graphs must survive into the store, and the dataset loader is
        // whole-document — so THIS is the one non-streaming path (documented on the module).
        let mut nq = String::new();
        reader.read_to_string(&mut nq).unwrap_or_else(|e| die(e));
        sparq_core::Graph::load_dataset(&nq, "nquads")
    } else {
        sparq_core::Graph::load_reader_parallel(reader, "ntriples")
    }
    .unwrap_or_else(|e| die(e));
    let secs = t.elapsed().as_secs_f64();
    let n = g.len() + g.named.iter().map(|(_, ng)| ng.len()).sum::<usize>();
    eprintln!(
        "loaded {n} {kind} in {secs:.3}s ({:.2} M/s) from tabular import",
        n as f64 / secs / 1e6
    );
    if let Some(q) = &flags.query {
        let count_only = args.iter().any(|a| a == "--count");
        crate::emit_query_results(&g, q, count_only, crate::out_format_flag(args));
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn rows(csv: &str) -> Vec<Vec<String>> {
        CsvRows::new(Cursor::new(csv.as_bytes().to_vec()), b',')
            .collect::<Result<Vec<_>, _>>()
            .expect("csv parses")
    }

    fn rows_err(csv: &str) -> String {
        CsvRows::new(Cursor::new(csv.as_bytes().to_vec()), b',')
            .collect::<Result<Vec<Vec<String>>, _>>()
            .expect_err("csv must fail")
    }

    // ---- CSV reader --------------------------------------------------------------------

    #[test]
    fn csv_basic_lf_and_crlf_and_no_trailing_newline() {
        assert_eq!(rows("a,b\r\n1,2\n3,4"), vec![vec!["a", "b"], vec!["1", "2"], vec!["3", "4"]]);
    }

    #[test]
    fn csv_quoted_separator_newline_and_escaped_quote() {
        let got = rows("name,quote\n\"Doe, Jane\",\"said \"\"hi\"\"\nbye\"\n");
        assert_eq!(got, vec![vec!["name", "quote"], vec!["Doe, Jane", "said \"hi\"\nbye"]]);
    }

    #[test]
    fn csv_bom_stripped_and_blank_lines_skipped() {
        assert_eq!(rows("\u{feff}a,b\n\n1,2\n"), vec![vec!["a", "b"], vec!["1", "2"]]);
    }

    #[test]
    fn csv_empty_fields_kept() {
        assert_eq!(rows("a,b,c\n,,\n"), vec![vec!["a", "b", "c"], vec!["", "", ""]]);
    }

    #[test]
    fn csv_custom_separator() {
        let got: Vec<Vec<String>> = CsvRows::new(Cursor::new(b"a\tb\n1\t2\n".to_vec()), b'\t')
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(got, vec![vec!["a", "b"], vec!["1", "2"]]);
    }

    #[test]
    fn csv_loud_errors() {
        assert!(rows_err("a\"b\n").contains("stray"));
        assert!(rows_err("\"abc\n").contains("unterminated"));
        assert!(rows_err("a\rb\n").contains("bare CR"));
        assert!(rows_err("\"a\"x\n").contains("after the closing"));
        assert!(CsvRows::new(Cursor::new(vec![b'a', 0xFF, b'\n']), b',')
            .collect::<Result<Vec<Vec<String>>, _>>()
            .expect_err("invalid utf8")
            .contains("UTF-8"));
    }

    // ---- term helpers ------------------------------------------------------------------

    #[test]
    fn datatype_inference_exact() {
        let xsd = "http://www.w3.org/2001/XMLSchema#";
        assert_eq!(infer_datatype("42"), Some(format!("{xsd}integer")).as_deref());
        assert_eq!(infer_datatype("-7"), Some(format!("{xsd}integer")).as_deref());
        assert_eq!(infer_datatype("3.25"), Some(format!("{xsd}decimal")).as_deref());
        assert_eq!(infer_datatype("-0.5"), Some(format!("{xsd}decimal")).as_deref());
        assert_eq!(infer_datatype("6.02e23"), Some(format!("{xsd}double")).as_deref());
        assert_eq!(infer_datatype("1E-9"), Some(format!("{xsd}double")).as_deref());
        assert_eq!(infer_datatype("true"), Some(format!("{xsd}boolean")).as_deref());
        for s in ["", "hello", "1.2.3", "1e", "e9", ".", "+", "TRUE", "NaN", "INF", "0x1F", "12 "] {
            assert_eq!(infer_datatype(s), None, "{s:?} must stay a plain string");
        }
    }

    #[test]
    fn iri_safe_percent_encodes() {
        assert_eq!(iri_safe("Hello World/42?"), "Hello%20World%2F42%3F");
        assert_eq!(iri_safe("a-b.c_d~e"), "a-b.c_d~e");
        assert_eq!(iri_safe("café"), "café"); // non-ASCII kept (IRI, not URI)
    }

    #[test]
    fn nt_literal_escaping_exact() {
        assert_eq!(escape_literal("a\"b\\c\nd\te\r"), "a\\\"b\\\\c\\nd\\te\\r");
        assert_eq!(escape_literal("bell\u{7}"), "bell\\u0007");
    }

    #[test]
    fn blank_label_injective() {
        assert_eq!(blank_label("a b"), "a_20b");
        assert_ne!(blank_label("a_20b"), blank_label("a b")); // '_' itself is escaped
        assert_eq!(blank_label(""), "_empty");
    }

    #[test]
    fn check_iri_rejects_relative_and_bad_chars() {
        assert!(check_iri("http://example.com/x").is_ok());
        assert!(check_iri("row/1").is_err());
        assert!(check_iri("http://e.com/a b").is_err());
    }

    // ---- direct mapping ----------------------------------------------------------------

    fn direct_nt(csv: &str, opts: &DirectOpts, table: &str) -> Vec<String> {
        let mut all = CsvRows::new(Cursor::new(csv.as_bytes().to_vec()), b',');
        let header = all.next().unwrap().unwrap();
        let map = compile_direct(&header, table, opts).unwrap();
        let mr = MappedRows { rows: Box::new(all), map, row_num: 0, failed: false };
        let mut lines: Vec<String> = Vec::new();
        for chunk in mr {
            lines.extend(chunk.unwrap().lines().map(str::to_owned));
        }
        lines.sort();
        lines
    }

    fn dopts() -> DirectOpts {
        DirectOpts {
            base: "http://example.com/".into(),
            template: None,
            class: None,
            infer: true,
            provenance: false,
        }
    }

    #[test]
    fn direct_mapping_default_template_class_and_inference() {
        let lines = direct_nt("name,age\nalice,34\n", &dopts(), "people");
        assert_eq!(
            lines,
            vec![
                "<http://example.com/people/row/1> <http://example.com/people#age> \"34\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                "<http://example.com/people/row/1> <http://example.com/people#name> \"alice\" .",
                "<http://example.com/people/row/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.com/people> .",
            ]
        );
    }

    #[test]
    fn direct_mapping_custom_template_class_none_no_infer_null_cell() {
        let opts = DirectOpts {
            base: "http://example.com/".into(),
            template: Some("http://example.com/emp/{ID}".into()),
            class: Some(None),
            infer: false,
            provenance: false,
        };
        let lines = direct_nt("ID,name,age\n7,ann,\n", &opts, "emp");
        assert_eq!(
            lines,
            vec![
                // age is NULL → no triple; no rdf:type; "7" stays a plain string under --no-infer
                "<http://example.com/emp/7> <http://example.com/emp#ID> \"7\" .",
                "<http://example.com/emp/7> <http://example.com/emp#name> \"ann\" .",
            ]
        );
    }

    #[test]
    fn direct_mapping_iri_safe_encoding_in_predicates_and_subjects() {
        let opts = DirectOpts {
            base: "http://example.com/".into(),
            template: Some("http://example.com/x/{full name}".into()),
            class: Some(None),
            infer: true,
            provenance: false,
        };
        let lines = direct_nt("full name\nJane Doe\n", &opts, "my table");
        assert_eq!(
            lines,
            vec!["<http://example.com/x/Jane%20Doe> <http://example.com/my%20table#full%20name> \"Jane Doe\" ."]
        );
    }

    #[test]
    fn direct_mapping_header_errors() {
        assert!(compile_direct(&["a".into(), "a".into()], "t", &dopts()).unwrap_err().contains("duplicate"));
        assert!(compile_direct(&["a".into(), String::new()], "t", &dopts()).unwrap_err().contains("empty"));
    }

    #[test]
    fn ragged_row_is_loud() {
        let mut all = CsvRows::new(Cursor::new(b"a,b\n1\n".to_vec()), b',');
        let header = all.next().unwrap().unwrap();
        let map = compile_direct(&header, "t", &dopts()).unwrap();
        let err = MappedRows { rows: Box::new(all), map, row_num: 0, failed: false }
            .collect::<Result<Vec<_>, _>>()
            .unwrap_err();
        assert!(err.contains("ragged"), "{err}");
    }

    #[test]
    fn template_parsing_escapes_and_errors() {
        let hdr = vec!["a".to_string()];
        assert!(parse_template("x\\{y\\}z{a}", &hdr, false).is_ok());
        assert!(parse_template("{missing}", &hdr, false).unwrap_err().contains("not found"));
        assert!(parse_template("{a", &hdr, false).unwrap_err().contains("unclosed"));
        assert!(parse_template("a}b", &hdr, false).unwrap_err().contains("unbalanced"));
        assert!(parse_template("{_row}", &hdr, false).unwrap_err().contains("_row"));
        assert!(parse_template("{_row}", &hdr, true).is_ok());
    }

    #[test]
    fn resolve_col_exact_ci_delimited_ambiguous() {
        let hdr: Vec<String> = ["Name", "NAME", "Age"].map(String::from).to_vec();
        assert_eq!(resolve_col("Name", &hdr).unwrap(), 0); // exact wins
        assert_eq!(resolve_col("age", &hdr).unwrap(), 2); // case-insensitive fallback
        assert!(resolve_col("name", &hdr).unwrap_err().contains("ambiguous"));
        assert_eq!(resolve_col("\"NAME\"", &hdr).unwrap(), 1); // delimited = exact only
        assert!(resolve_col("\"name\"", &hdr).unwrap_err().contains("not found"));
    }

    // ---- R2RML -------------------------------------------------------------------------

    /// Run an R2RML mapping over in-memory CSV tables; sorted unique N-Triples/N-Quads out.
    fn r2rml_nt(mapping: &str, tables: &[(&str, &str)]) -> Result<Vec<String>, String> {
        r2rml_nt_prov(mapping, tables, false)
    }

    /// [`r2rml_nt`] with the `--row-provenance` prefix wired in (`http://example.com/<table>/row/`).
    fn r2rml_nt_prov(
        mapping: &str,
        tables: &[(&str, &str)],
        provenance: bool,
    ) -> Result<Vec<String>, String> {
        let m = parse_r2rml(mapping)?;
        let owned: Vec<(String, String)> =
            tables.iter().map(|(n, c)| ((*n).to_owned(), (*c).to_owned())).collect();
        let mut open = |table: &str| -> Result<(Vec<String>, RowIter), String> {
            let (_, csv) = owned
                .iter()
                .find(|(n, _)| n == table)
                .ok_or_else(|| format!("test: no CSV for table {table}"))?;
            let mut rows = CsvRows::new(Cursor::new(csv.as_bytes().to_vec()), b',');
            let header = rows.next().ok_or("empty csv")??;
            Ok((header, Box::new(rows) as RowIter))
        };
        let mut lines: Vec<String> = Vec::new();
        for (tm_key, table) in &m.tms {
            let (header, rows) = open(table)?;
            let prov =
                provenance.then(|| row_iri_prefix("http://example.com/", table)).transpose()?;
            let map = compile_tm(&m, tm_key, &header, None, prov.as_deref(), &mut open)?;
            for chunk in (MappedRows { rows, map, row_num: 0, failed: false }) {
                lines.extend(chunk?.lines().map(str::to_owned));
            }
        }
        lines.sort();
        lines.dedup();
        Ok(lines)
    }

    const PREAMBLE: &str = "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n@prefix ex: <http://example.com/> .\n@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";

    #[test]
    fn r2rml_template_subject_class_and_column_object() {
        let mapping = format!(
            "{PREAMBLE}\
             ex:TM a rr:TriplesMap ;\n\
               rr:logicalTable [ rr:tableName \"student\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/{{ID}}/{{Name}}\" ; rr:class ex:Student ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column \"Name\" ] ] .\n"
        );
        let got = r2rml_nt(&mapping, &[("student", "ID,Name\n10,Venus Williams\n")]).unwrap();
        assert_eq!(
            got,
            vec![
                "<http://example.com/10/Venus%20Williams> <http://example.com/name> \"Venus Williams\" .",
                "<http://example.com/10/Venus%20Williams> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.com/Student> .",
            ]
        );
    }

    #[test]
    fn r2rml_datatype_language_constant_and_null() {
        let mapping = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:age ; rr:objectMap [ rr:column \"age\" ; rr:datatype xsd:integer ] ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column \"label\" ; rr:language \"en\" ] ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:src ; rr:object \"csv\" ] .\n"
        );
        let got = r2rml_nt(&mapping, &[("t", "id,age,label\n1,33,alpha\n2,,beta\n")]).unwrap();
        assert_eq!(
            got,
            vec![
                "<http://example.com/r/1> <http://example.com/age> \"33\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                "<http://example.com/r/1> <http://example.com/label> \"alpha\"@en .",
                "<http://example.com/r/1> <http://example.com/src> \"csv\" .",
                // row 2: age is NULL → that triple only is skipped
                "<http://example.com/r/2> <http://example.com/label> \"beta\"@en .",
                "<http://example.com/r/2> <http://example.com/src> \"csv\" .",
            ]
        );
    }

    #[test]
    fn r2rml_column_iri_blank_node_and_predicate_map() {
        let mapping = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:column \"who\" ; rr:termType rr:IRI ] ;\n\
               rr:predicateObjectMap [\n\
                 rr:predicateMap [ rr:template \"http://example.com/p/{{prop}}\" ] ;\n\
                 rr:objectMap [ rr:column \"tag\" ; rr:termType rr:BlankNode ]\n\
               ] .\n"
        );
        let got = r2rml_nt(&mapping, &[("t", "who,prop,tag\nhttp://example.com/alice,knows,x 1\n")]).unwrap();
        assert_eq!(got, vec!["<http://example.com/alice> <http://example.com/p/knows> _:x_201 ."]);
    }

    #[test]
    fn r2rml_null_subject_skips_row_and_default_literal_termtype() {
        let mapping = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:v ; rr:objectMap [ rr:column \"v\" ] ] .\n"
        );
        let got = r2rml_nt(&mapping, &[("t", "id,v\n,gone\n1,kept\n")]).unwrap();
        assert_eq!(got, vec!["<http://example.com/r/1> <http://example.com/v> \"kept\" ."]);
    }

    #[test]
    fn r2rml_empty_mapping_generates_nothing() {
        let got = r2rml_nt(PREAMBLE, &[]).unwrap();
        assert!(got.is_empty());
    }

    /// SQL-connection R2RML stays a NON-GOAL: every construct that only makes sense against a
    /// live SQL connection is still a loud, fail-closed error — never a silent skip.
    #[test]
    fn r2rml_unsupported_constructs_fail_closed() {
        for (frag, needle) in [
            ("ex:TM rr:logicalTable [ rr:sqlQuery \"SELECT 1\" ] .", "rr:sqlQuery"),
            (
                "ex:TM rr:logicalTable [ rr:tableName \"t\" ; rr:sqlVersion ex:SQL2008 ] .",
                "rr:sqlVersion",
            ),
            (
                "ex:TM rr:logicalTable [ rr:tableName \"t\" ] ; rr:subjectMap [ rr:template \"http://e/{{x}}\" ; rr:inverseExpression \"{x} = 1\" ] .",
                "rr:inverseExpression",
            ),
        ] {
            let err = r2rml_nt(&format!("{PREAMBLE}{frag}\n"), &[("t", "x\n1\n")]).unwrap_err();
            assert!(err.contains(needle), "{err}");
        }
    }

    #[test]
    fn r2rml_shape_errors() {
        // both template and column
        let err = r2rml_nt(
            &format!(
                "{PREAMBLE}ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
                 rr:subjectMap [ rr:template \"http://e/{{x}}\" ; rr:column \"x\" ] .\n"
            ),
            &[("t", "x\n1\n")],
        )
        .unwrap_err();
        assert!(err.contains("exactly one"), "{err}");
        // datatype + language together
        let err = r2rml_nt(
            &format!(
                "{PREAMBLE}ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
                 rr:subjectMap [ rr:template \"http://e/{{x}}\" ] ;\n\
                 rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"x\" ; rr:datatype xsd:int ; rr:language \"en\" ] ] .\n"
            ),
            &[("t", "x\n1\n")],
        )
        .unwrap_err();
        assert!(err.contains("both rr:datatype and rr:language"), "{err}");
        // missing subject
        let err = r2rml_nt(
            &format!("{PREAMBLE}ex:TM rr:logicalTable [ rr:tableName \"t\" ] .\n"),
            &[("t", "x\n1\n")],
        )
        .unwrap_err();
        assert!(err.contains("no rr:subjectMap"), "{err}");
    }

    #[test]
    fn r2rml_pom_cartesian_two_predicates_two_objects() {
        let mapping = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ] ;\n\
               rr:predicateObjectMap [\n\
                 rr:predicate ex:p1 ; rr:predicate ex:p2 ;\n\
                 rr:objectMap [ rr:column \"a\" ] ; rr:objectMap [ rr:column \"b\" ]\n\
               ] .\n"
        );
        let got = r2rml_nt(&mapping, &[("t", "id,a,b\n1,x,y\n")]).unwrap();
        assert_eq!(got.len(), 4, "{got:?}");
        for (p, o) in [("p1", "x"), ("p1", "y"), ("p2", "x"), ("p2", "y")] {
            assert!(got.contains(&format!("<http://example.com/r/1> <http://example.com/{p}> \"{o}\" .")), "{got:?}");
        }
    }

    // ---- R2RML joins (sq-u1z86) --------------------------------------------------------

    const SPORT_CSV: &str = "ID,Name\n110,Tennis\n111,Football\n";
    /// Row 10 joins, row 11's key matches no parent, row 12's key is NULL.
    const STUDENT_CSV: &str = "ID,Name,Sport\n10,Venus,110\n11,Fred,999\n12,Ann,\n";

    fn join_mapping(condition: &str) -> String {
        format!(
            "{PREAMBLE}\
             ex:Sport rr:logicalTable [ rr:tableName \"sport\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/sport/{{ID}}\" ] .\n\
             ex:Student rr:logicalTable [ rr:tableName \"student\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/student/{{ID}}\" ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:plays ;\n\
                 rr:objectMap [ rr:parentTriplesMap ex:Sport ; {condition} ] ] .\n"
        )
    }

    /// The headline join: the child's `Sport` column keys into the parent's `ID`, and the
    /// generated object is the PARENT's subject IRI — with SQL NULL/no-match semantics.
    #[test]
    fn r2rml_join_parent_triples_map() {
        let got = r2rml_nt(
            &join_mapping("rr:joinCondition [ rr:child \"Sport\" ; rr:parent \"ID\" ]"),
            &[("sport", SPORT_CSV), ("student", STUDENT_CSV)],
        )
        .unwrap();
        assert_eq!(
            got,
            vec![
                // ONLY student 10 joins: 11's key (999) matches nothing, 12's key is NULL.
                "<http://example.com/student/10> <http://example.com/plays> <http://example.com/sport/110> .",
            ]
        );
    }

    /// A two-condition join is a TUPLE join (both columns must match), and a parent row whose
    /// key cell is NULL never enters the index.
    #[test]
    fn r2rml_join_multi_condition_and_null_parent_key() {
        let mapping = join_mapping(
            "rr:joinCondition [ rr:child \"a\" ; rr:parent \"x\" ] ; rr:joinCondition [ rr:child \"b\" ; rr:parent \"y\" ]",
        )
        .replace("{ID}\" ] .\nex:Student", "{x}-{y}\" ] .\nex:Student")
        .replace("student/{ID}", "student/{id}");
        let got = r2rml_nt(
            &mapping,
            &[
                // (1,2) is joinable; (1,) has a NULL second key so it is never indexed.
                ("sport", "x,y\n1,2\n1,\n"),
                // s1 matches (1,2); s2's (1,9) matches nothing; s3 flips the tuple order.
                ("student", "id,a,b\ns1,1,2\ns2,1,9\ns3,2,1\n"),
            ],
        )
        .unwrap();
        assert_eq!(
            got,
            vec!["<http://example.com/student/s1> <http://example.com/plays> <http://example.com/sport/1-2> ."]
        );
    }

    /// Join shapes that cannot be executed honestly are loud, never silently empty.
    #[test]
    fn r2rml_join_shape_errors() {
        // no join condition -> the SQL cross-join form, deliberately out of scope
        let err = r2rml_nt(&join_mapping(""), &[("sport", SPORT_CSV), ("student", STUDENT_CSV)])
            .unwrap_err();
        assert!(err.contains("rr:joinCondition"), "{err}");
        // a referencing object map may not also be a term map
        let err = r2rml_nt(
            &join_mapping("rr:column \"Sport\" ; rr:joinCondition [ rr:child \"Sport\" ; rr:parent \"ID\" ]"),
            &[("sport", SPORT_CSV), ("student", STUDENT_CSV)],
        )
        .unwrap_err();
        assert!(err.contains("must not also have rr:column"), "{err}");
        // rr:parent naming a column the parent header does not have
        let err = r2rml_nt(
            &join_mapping("rr:joinCondition [ rr:child \"Sport\" ; rr:parent \"nope\" ]"),
            &[("sport", SPORT_CSV), ("student", STUDENT_CSV)],
        )
        .unwrap_err();
        assert!(err.contains("\"nope\" not found"), "{err}");
        // rr:parentTriplesMap pointing at something that is not a triples map
        let err = r2rml_nt(
            &join_mapping("rr:joinCondition [ rr:child \"Sport\" ; rr:parent \"ID\" ]")
                .replace("rr:parentTriplesMap ex:Sport", "rr:parentTriplesMap ex:Nothing"),
            &[("sport", SPORT_CSV), ("student", STUDENT_CSV)],
        )
        .unwrap_err();
        assert!(err.contains("is not a triples map"), "{err}");
    }

    // ---- R2RML named graphs (sq-u1z86) -------------------------------------------------

    /// Compile (without emitting) every triples map — for asserting the N-Triples/N-Quads switch.
    fn r2rml_maps(mapping: &str, tables: &[(&str, &str)]) -> Result<Vec<CompiledMap>, String> {
        let m = parse_r2rml(mapping)?;
        let owned: Vec<(String, String)> =
            tables.iter().map(|(n, c)| ((*n).to_owned(), (*c).to_owned())).collect();
        let mut open = |table: &str| -> Result<(Vec<String>, RowIter), String> {
            let (_, csv) = owned.iter().find(|(n, _)| n == table).ok_or("test: no CSV")?;
            let mut rows = CsvRows::new(Cursor::new(csv.as_bytes().to_vec()), b',');
            let header = rows.next().ok_or("empty csv")??;
            Ok((header, Box::new(rows) as RowIter))
        };
        let mut out = Vec::new();
        for (tm_key, table) in &m.tms {
            let (header, _) = open(table)?;
            out.push(compile_tm(&m, tm_key, &header, None, None, &mut open)?);
        }
        Ok(out)
    }

    /// Subject-map graphs scope the class triples AND union into every predicate-object map's;
    /// a per-row `rr:graphMap` template gives one named graph per row.
    #[test]
    fn r2rml_graph_maps_emit_quads() {
        let mapping = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ; rr:class ex:C ; rr:graph ex:g1 ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ;\n\
                 rr:graphMap [ rr:template \"http://example.com/g/{{cat}}\" ] ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column \"v\" ] ] .\n"
        );
        let tables = [("t", "id,v,cat\n1,x,red\n")];
        assert!(r2rml_maps(&mapping, &tables).unwrap()[0].uses_graphs());
        assert_eq!(
            r2rml_nt(&mapping, &tables).unwrap(),
            vec![
                // ex:p lands in BOTH the subject graph and the per-row graph (the union)…
                "<http://example.com/r/1> <http://example.com/p> \"x\" <http://example.com/g/red> .",
                "<http://example.com/r/1> <http://example.com/p> \"x\" <http://example.com/g1> .",
                // …ex:q and the class triple only in the subject-map graph.
                "<http://example.com/r/1> <http://example.com/q> \"x\" <http://example.com/g1> .",
                "<http://example.com/r/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.com/C> <http://example.com/g1> .",
            ]
        );
    }

    /// `rr:defaultGraph` names the default graph (a plain triple), and it takes part in the
    /// union like any other graph — so a per-row graph map next to it emits BOTH.
    #[test]
    fn r2rml_default_graph_constant_joins_the_union() {
        let mapping = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ; rr:graph rr:defaultGraph ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ;\n\
                 rr:graphMap [ rr:template \"http://example.com/g/{{cat}}\" ] ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column \"v\" ] ] .\n"
        );
        // Row 1 has a graph; row 2's `cat` is NULL, so the predicate-object map generates no
        // graph — but the subject map's default graph survives the union, so both statements
        // still land there.
        let got = r2rml_nt(&mapping, &[("t", "id,v,cat\n1,x,red\n2,y,\n")]).unwrap();
        assert_eq!(
            got,
            vec![
                "<http://example.com/r/1> <http://example.com/p> \"x\" .",
                "<http://example.com/r/1> <http://example.com/p> \"x\" <http://example.com/g/red> .",
                "<http://example.com/r/1> <http://example.com/q> \"x\" .",
                "<http://example.com/r/2> <http://example.com/p> \"y\" .",
                "<http://example.com/r/2> <http://example.com/q> \"y\" .",
            ]
        );
    }

    /// A NULL graph map contributes NOTHING to the union — it never erases a graph generated
    /// by a sibling graph map or by the other side of the subject/predicate-object union.
    #[test]
    fn r2rml_null_graph_map_does_not_erase_the_union() {
        // (a) NULL predicate-object graph map alongside a valid NAMED subject graph.
        let subject_named = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ; rr:class ex:C ; rr:graph ex:g1 ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ;\n\
                 rr:graphMap [ rr:template \"http://example.com/g/{{cat}}\" ] ] .\n"
        );
        assert_eq!(
            r2rml_nt(&subject_named, &[("t", "id,v,cat\n1,x,\n")]).unwrap(),
            vec![
                "<http://example.com/r/1> <http://example.com/p> \"x\" <http://example.com/g1> .",
                "<http://example.com/r/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.com/C> <http://example.com/g1> .",
            ]
        );
        // (b) NULL subject graph map alongside a valid predicate-object graph: the class triple
        // is scoped by the subject set alone, so it drops, but the ex:p statement keeps ex:g2.
        let pom_named = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ; rr:class ex:C ;\n\
                 rr:graphMap [ rr:template \"http://example.com/g/{{cat}}\" ] ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ; rr:graph ex:g2 ] .\n"
        );
        assert_eq!(
            r2rml_nt(&pom_named, &[("t", "id,v,cat\n1,x,\n")]).unwrap(),
            vec!["<http://example.com/r/1> <http://example.com/p> \"x\" <http://example.com/g2> ."]
        );
        // (c) TWO subject graph maps, only one NULL: the survivor still scopes everything.
        let two_maps = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ; rr:graph ex:g1 ;\n\
                 rr:graphMap [ rr:template \"http://example.com/g/{{cat}}\" ] ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ] .\n"
        );
        assert_eq!(
            r2rml_nt(&two_maps, &[("t", "id,v,cat\n1,x,\n")]).unwrap(),
            vec!["<http://example.com/r/1> <http://example.com/p> \"x\" <http://example.com/g1> ."]
        );
    }

    /// When EVERY declared graph map generates NULL the statement is unscoped and dropped — a
    /// declared-but-NULL graph set is not the same thing as no graph maps at all, so there is
    /// no silent fallback into the default graph.
    #[test]
    fn r2rml_all_null_graph_maps_drop_the_statement() {
        let mapping = format!(
            "{PREAMBLE}\
             ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
               rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ; rr:class ex:C ;\n\
                 rr:graphMap [ rr:template \"http://example.com/g/{{cat}}\" ] ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ] ;\n\
               rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column \"v\" ] ;\n\
                 rr:graphMap [ rr:template \"http://example.com/h/{{cat}}\" ] ] .\n"
        );
        // Row 1 keeps everything; row 2's NULL `cat` empties BOTH scopes, so it emits nothing.
        assert_eq!(
            r2rml_nt(&mapping, &[("t", "id,v,cat\n1,x,red\n2,y,\n")]).unwrap(),
            vec![
                "<http://example.com/r/1> <http://example.com/p> \"x\" <http://example.com/g/red> .",
                "<http://example.com/r/1> <http://example.com/q> \"x\" <http://example.com/g/red> .",
                "<http://example.com/r/1> <http://example.com/q> \"x\" <http://example.com/h/red> .",
                "<http://example.com/r/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.com/C> <http://example.com/g/red> .",
            ]
        );
    }

    /// A graph-map-free mapping keeps the N-Triples fast path (no quad column at all).
    #[test]
    fn r2rml_without_graph_maps_stays_ntriples() {
        let mapping = format!(
            "{PREAMBLE}ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
             rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ] ;\n\
             rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ] .\n"
        );
        let tables = [("t", "id,v\n1,x\n")];
        assert!(!r2rml_maps(&mapping, &tables).unwrap()[0].uses_graphs());
        assert_eq!(
            r2rml_nt(&mapping, &tables).unwrap(),
            vec!["<http://example.com/r/1> <http://example.com/p> \"x\" ."]
        );
    }

    /// Graph maps must generate IRIs — a literal or blank-node graph is a loud error.
    #[test]
    fn r2rml_graph_map_must_be_an_iri() {
        for (frag, needle) in [
            ("rr:graphMap [ rr:column \"v\" ; rr:termType rr:BlankNode ]", "cannot be a blank node"),
            ("rr:graph \"g\"", "only valid in object position"),
        ] {
            let err = r2rml_nt(
                &format!(
                    "{PREAMBLE}ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
                     rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ; {frag} ] ;\n\
                     rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ] .\n"
                ),
                &[("t", "id,v\n1,x\n")],
            )
            .unwrap_err();
            assert!(err.contains(needle), "{err}");
        }
    }

    // ---- row provenance (sq-u1z86) -----------------------------------------------------

    /// `--row-provenance` links a custom-template subject back to the exact source row.
    #[test]
    fn row_provenance_direct_mapping() {
        let opts = DirectOpts {
            base: "http://example.com/".into(),
            template: Some("http://example.com/emp/{ID}".into()),
            class: Some(None),
            infer: true,
            provenance: true,
        };
        assert_eq!(
            direct_nt("ID\n7\n", &opts, "people"),
            vec![
                "<http://example.com/emp/7> <http://example.com/people#ID> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                "<http://example.com/emp/7> <http://www.w3.org/ns/prov#wasDerivedFrom> <http://example.com/people/row/1> .",
            ]
        );
    }

    /// The same option works under R2RML, and follows the row into its NAMED graph.
    #[test]
    fn row_provenance_r2rml_follows_the_graph_map() {
        let mapping = format!(
            "{PREAMBLE}ex:TM rr:logicalTable [ rr:tableName \"t\" ] ;\n\
             rr:subjectMap [ rr:template \"http://example.com/r/{{id}}\" ; rr:graph ex:g ] ;\n\
             rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column \"v\" ] ] .\n"
        );
        let got = r2rml_nt_prov(&mapping, &[("t", "id,v\na,x\nb,y\n")], true).unwrap();
        assert_eq!(
            got,
            vec![
                "<http://example.com/r/a> <http://example.com/p> \"x\" <http://example.com/g> .",
                "<http://example.com/r/a> <http://www.w3.org/ns/prov#wasDerivedFrom> <http://example.com/t/row/1> <http://example.com/g> .",
                "<http://example.com/r/b> <http://example.com/p> \"y\" <http://example.com/g> .",
                "<http://example.com/r/b> <http://www.w3.org/ns/prov#wasDerivedFrom> <http://example.com/t/row/2> <http://example.com/g> .",
            ]
        );
    }

    // ---- plumbing ----------------------------------------------------------------------

    #[test]
    fn table_stem_strips_dir_compression_and_format() {
        assert_eq!(table_stem("people.csv"), "people");
        assert_eq!(table_stem("/a/b/people.csv.gz"), "people");
        assert_eq!(table_stem("x.csv.zst"), "x");
        assert_eq!(table_stem("noext"), "noext");
    }

    #[test]
    fn nt_reader_streams_and_surfaces_errors() {
        let ok: ChunkIter = Box::new(vec![Ok("a\n".to_string()), Ok(String::new()), Ok("b\n".to_string())].into_iter());
        let mut out = String::new();
        NtReader::new(vec![ok]).read_to_string(&mut out).unwrap();
        assert_eq!(out, "a\nb\n");
        let bad: ChunkIter = Box::new(vec![Ok("a\n".to_string()), Err("boom".to_string())].into_iter());
        let err = NtReader::new(vec![bad]).read_to_string(&mut String::new()).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    /// End-to-end into the engine: direct-mapped CSV chunks load through
    /// `load_reader_parallel` and are queryable.
    #[test]
    fn direct_mapping_loads_into_graph_and_queries() {
        let csv = "name,age\nalice,34\nbob,28\n";
        let mut all = CsvRows::new(Cursor::new(csv.as_bytes().to_vec()), b',');
        let header = all.next().unwrap().unwrap();
        let map = compile_direct(&header, "people", &dopts()).unwrap();
        let src: ChunkIter = Box::new(MappedRows { rows: Box::new(all), map, row_num: 0, failed: false });
        let g = sparq_core::Graph::load_reader_parallel(NtReader::new(vec![src]), "ntriples").unwrap();
        assert_eq!(g.len(), 6); // 2 rows × (2 columns + rdf:type)
        let r = sparq_engine::query(
            &g,
            "SELECT ?n WHERE { ?s <http://example.com/people#age> ?a . ?s <http://example.com/people#name> ?n . FILTER(?a > 30) }",
        )
        .unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0], Some(oxrdf::Term::Literal(oxrdf::Literal::new_simple_literal("alice"))));
    }
}
