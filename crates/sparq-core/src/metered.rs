//! Bounded RDF ingestion with streaming or bulk admission before exposing a graph.
use crate::dict::Dict;
use crate::meter::{Dim, Ledger, LoadError, TermRef};
use crate::Graph;
use oxrdf::{GraphName, Term};
use oxttl::{NQuadsParser, NTriplesParser, TriGParser, TurtleParser};
use std::collections::{HashMap, HashSet};

impl Graph {
    /// Load a dataset under `ledger`, preserving named graphs.
    /// Charges source bytes before parsing, then term bytes and facts before retaining
    /// each row on the serial path. Larger source-bounded N-Quads use the ordinary
    /// parallel loader followed by bulk admission before the graph is exposed.
    /// Tighter limits retain streaming refusal behavior. No graph is returned on failure.
    /// Formats and aliases match [`Self::load_dataset`].
    pub fn load_metered(text: &str, format: &str, ledger: &mut Ledger) -> Result<Self, LoadError> {
        load(text, format, None, true, true, ledger)
    }

    /// Load text whose external source was already charged by the caller.
    /// Intended for a source plus separately bounded generated facts. The caller
    /// must charge source bytes before allocating or appending generated text.
    /// Terms, facts and buffers retain the ordinary metered loader's checks.
    pub fn load_metered_precharged(
        text: &str,
        format: &str,
        ledger: &mut Ledger,
    ) -> Result<Self, LoadError> {
        load(text, format, None, true, false, ledger)
    }

    /// Project an already admitted dataset into one metered union graph.
    /// Charges retained terms and auxiliary rows, without charging source bytes or
    /// facts already admitted during ingestion. Named graph labels are omitted, as for a folded load.
    pub fn union_metered(&self, ledger: &mut Ledger) -> Result<Self, LoadError> {
        let mut dict = Dict::new();
        let mut rows = Vec::new();
        for graph in std::iter::once(self).chain(self.named().iter().map(|(_, graph)| graph)) {
            for row in graph.iter_ids() {
                for id in row {
                    ledger.stored_term(graph.dict(), id)?;
                }
                ledger.reserve(&mut rows, 1, Dim::Nodes)?;
                let mut projected = [0; 3];
                for (position, id) in row.into_iter().enumerate() {
                    let term = graph.dict().term(id);
                    projected[position] = dict.intern(&term);
                }
                rows.push(projected);
            }
        }
        dict.shrink_term_capacity();
        Ok(Self::from_parts(dict, rows))
    }

    /// Metered counterpart of [`Self::load_str_with_base`], folding named graphs.
    /// The source, terms and rows share the caller's ledger with subsequent work.
    /// Respects its document window; call [`Ledger::open_document`] between documents.
    pub fn load_metered_with_base(
        text: &str,
        format: &str,
        base: &str,
        ledger: &mut Ledger,
    ) -> Result<Self, LoadError> {
        load(text, format, Some(base), false, true, ledger)
    }
}

fn load(
    text: &str,
    format: &str,
    base: Option<&str>,
    dataset: bool,
    charge_source: bool,
    ledger: &mut Ledger,
) -> Result<Graph, LoadError> {
    if charge_source {
        ledger.charge(Dim::SourceBytes, text.len())?;
    }
    if dataset {
        ledger.close_document();
    }
    #[cfg(feature = "parallel")]
    if dataset && crate::is_nquads_format(format) && parallel_budget_covers(text, ledger) {
        let mut source_rows = 0;
        let compatible = text.split(['\n', '\r']).all(|line| {
            let line = line.trim_start();
            if !line.is_empty() && !line.starts_with('#') {
                source_rows += 1;
            }
            // The streaming parser predates native version directives.
            !line.starts_with("VERSION")
        });
        if compatible && source_rows >= crate::store::SERIAL_PERM_ROWS {
            let threads = rayon::current_num_threads().max(1);
            let target = (threads * 4).min(text.len() / 4096 + 1).max(1);
            let chunks = Graph::parse_nquads_chunks_report(text.as_bytes(), target);
            return finish_parallel(text, chunks, ledger);
        }
    }
    load_serial(text, format, base, dataset, ledger)
}

fn load_serial(
    text: &str,
    format: &str,
    base: Option<&str>,
    dataset: bool,
    ledger: &mut Ledger,
) -> Result<Graph, LoadError> {
    #[cfg(test)]
    LOAD_PATHS.with(|paths| {
        let (serial, parallel) = paths.get();
        paths.set((serial + 1, parallel));
    });
    let (groups, _) =
        load_serial_parts(text, 0, format, base, dataset, ledger, HashMap::new())?;
    finish_serial(groups, ledger)
}

type Groups = Vec<(Option<Term>, Dict, Vec<[crate::dict::Id; 3]>)>;
type GroupIndex = HashMap<Option<Term>, usize>;

#[allow(clippy::too_many_arguments)]
fn load_serial_parts(
    source: &str,
    offset: usize,
    format: &str,
    base: Option<&str>,
    dataset: bool,
    ledger: &mut Ledger,
    mut index: GroupIndex,
) -> Result<(Groups, GroupIndex), LoadError> {
    let text = &source[offset..];
    #[cfg(test)]
    STREAMED_BYTES.with(|bytes| bytes.set(text.len()));
    let mut groups: Groups = Vec::new();
    let mut document_rows = HashSet::new();
    let mut push = |subject: oxrdf::NamedOrBlankNode,
                    predicate: oxrdf::NamedNode,
                    object: Term,
                    graph: GraphName|
     -> Result<(), LoadError> {
        let terms = [
            crate::subject_term(&subject),
            Term::NamedNode(predicate),
            object,
        ];
        if !dataset {
            if let Some((_, dict, _)) = groups.first() {
                let ids = terms.each_ref().map(|term| dict.lookup(term));
                if document_rows.contains(&ids) {
                    return Ok(());
                }
            }
        }
        for term in &terms {
            ledger.term(TermRef::from(term))?;
        }
        let name = if dataset {
            match graph {
                GraphName::DefaultGraph => None,
                GraphName::NamedNode(n) => Some(Term::NamedNode(n)),
                GraphName::BlankNode(b) => Some(Term::BlankNode(b)),
            }
        } else {
            None
        };
        if let Some(term) = &name {
            ledger.term(TermRef::from(term))?;
        }
        ledger.charge(Dim::Facts, 1)?;
        let slot = match index.get(&name) {
            Some(&slot) if slot != usize::MAX => slot,
            previous => {
                if previous.is_none() {
                    ledger.reserve(&mut groups, 1, Dim::Nodes)?;
                } else {
                    groups.try_reserve(1).map_err(|_| {
                        LoadError::Ledger(crate::meter::Exceeded::new(
                            Dim::Alloc,
                            usize::MAX,
                            1,
                        ))
                    })?;
                }
                index.try_reserve(1).map_err(|_| {
                    LoadError::Ledger(crate::meter::Exceeded::new(Dim::Alloc, usize::MAX, 1))
                })?;
                let slot = groups.len();
                index.insert(name.clone(), slot);
                groups.push((name, Dict::new(), Vec::new()));
                slot
            }
        };
        let (_, dict, rows) = &mut groups[slot];
        ledger.reserve(rows, 1, Dim::Nodes)?;
        let row = terms.each_ref().map(|term| dict.intern(term));
        if !dataset {
            document_rows.try_reserve(1).map_err(|_| {
                LoadError::Ledger(crate::meter::Exceeded::new(Dim::Alloc, usize::MAX, 1))
            })?;
            document_rows.insert(row);
        }
        rows.push(row);
        Ok(())
    };
    macro_rules! with_base {
        ($parser:expr) => {
            match base {
                Some(base) => $parser
                    .with_base_iri(base)
                    .map_err(|e| LoadError::Parse(e.to_string()))?,
                None => $parser,
            }
        };
    }
    macro_rules! quads {
        ($parser:expr) => {
            for quad in $parser.for_slice(text.as_bytes()) {
                let quad = quad.map_err(|e| LoadError::Parse(e.to_string()))?;
                push(quad.subject, quad.predicate, quad.object, quad.graph_name)?;
            }
        };
    }
    macro_rules! triples {
        ($parser:expr) => {
            for triple in $parser.for_slice(text.as_bytes()) {
                let triple = triple.map_err(|e| LoadError::Parse(e.to_string()))?;
                push(
                    triple.subject,
                    triple.predicate,
                    triple.object,
                    GraphName::DefaultGraph,
                )?;
            }
        };
    }
    match format {
        "nquads" | "n-quads" | "nq" | "application/n-quads" => {
            for quad in NQuadsParser::new().for_slice(text.as_bytes()) {
                let quad =
                    quad.map_err(|error| LoadError::Parse(error_at(error, source, offset)))?;
                push(quad.subject, quad.predicate, quad.object, quad.graph_name)?;
            }
        }
        "ntriples" | "n-triples" | "nt" | "application/n-triples" => {
            triples!(NTriplesParser::new())
        }
        "trig" | "application/trig" => quads!(with_base!(TriGParser::new())),
        _ if crate::is_turtle_format(format) => triples!(with_base!(TurtleParser::new())),
        #[cfg(feature = "jsonld")]
        _ if crate::is_jsonld_format(format) => {
            quads!(with_base!(oxjsonld::JsonLdParser::new()))
        }
        #[cfg(feature = "rdfxml")]
        _ if crate::is_rdfxml_format(format) => {
            triples!(with_base!(oxrdfxml::RdfXmlParser::new()))
        }
        _ => return Err(LoadError::Parse(crate::unknown_format_err(format))),
    }
    Ok((groups, index))
}

fn finish_serial(groups: Groups, ledger: &mut Ledger) -> Result<Graph, LoadError> {
    let mut graph = None;
    let mut named = Vec::new();
    for (name, mut dict, rows) in groups {
        dict.shrink_term_capacity();
        let part = Graph::from_parts(dict, rows);
        if let Some(name) = name {
            ledger.reserve(&mut named, 1, Dim::Nodes)?;
            named.push((name, part));
        } else {
            graph = Some(part);
        }
    }
    let mut graph = graph.unwrap_or_default();
    // Every graph is freshly built; reuse the admitted vector as the ordinary loaders do.
    graph.named = named;
    Ok(graph)
}

#[cfg(feature = "parallel")]
fn parallel_budget_covers(text: &str, ledger: &Ledger) -> bool {
    // These source-derived bounds cover every streaming charge. Tight callers
    // retain the streaming path, including its first-refusal and partial counters.
    let bytes = text.len().saturating_mul(64);
    ledger.remaining(Dim::Facts) >= text.len()
        && ledger.remaining(Dim::Bytes) >= bytes
        && ledger.limits().term_bytes >= bytes
        && ledger.remaining(Dim::Nodes) >= text.len().saturating_mul(4)
}

// The resumed parser starts at a line boundary. Preserve the streaming parser's
// original diagnostic, including CRLF and Unicode column locations.
fn error_at(error: oxttl::TurtleSyntaxError, source: &str, offset: usize) -> String {
    if offset == 0 {
        return error.to_string();
    }
    let prefix = &source.as_bytes()[..offset];
    let lines = prefix
        .iter()
        .enumerate()
        .filter(|&(i, byte)| {
            *byte == b'\r' || (*byte == b'\n' && (i == 0 || prefix[i - 1] != b'\r'))
        })
        .count() as u64;
    let range = error.location();
    let (start, end) = (range.start, range.end);
    if start.offset + 1 >= end.offset {
        format!(
            "Parser error at line {} column {}: {}",
            start.line + lines + 1,
            start.column + 1,
            error.message()
        )
    } else if start.line == end.line {
        format!(
            "Parser error at line {} between columns {} and {}: {}",
            start.line + lines + 1,
            start.column + 1,
            end.column + 1,
            error.message()
        )
    } else {
        format!(
            "Parser error between line {} column {} and line {} column {}: {}",
            start.line + lines + 1,
            start.column + 1,
            end.line + lines + 1,
            end.column + 1,
            error.message()
        )
    }
}

#[cfg(feature = "parallel")]
#[derive(Default)]
struct ParsedTotals {
    bytes: usize,
    largest: usize,
    facts: usize,
    groups: usize,
    named: usize,
}

#[cfg(feature = "parallel")]
fn parsed_totals(
    chunks: &[(usize, crate::nt::QuadChunk)],
) -> Result<(ParsedTotals, GroupIndex), LoadError> {
    let mut totals = ParsedTotals::default();
    let mut index = HashMap::new();
    for (_, chunk) in chunks {
        for (name, dict, rows) in &chunk.buckets {
            if rows.is_empty() {
                continue;
            }
            let term = name.as_ref().map(crate::nt::GraphKey::to_term);
            let name_size = match &term {
                Some(Term::NamedNode(n)) => {
                    crate::meter::TERM_NODE_BYTES.saturating_add(n.as_str().len())
                }
                Some(Term::BlankNode(n)) => {
                    crate::meter::TERM_NODE_BYTES.saturating_add(n.as_str().len())
                }
                _ => 0,
            };
            index.insert(term, usize::MAX);
            totals.largest = totals.largest.max(name_size);
            // Raw rows retain source occurrences, including cross-chunk duplicates.
            for row in rows {
                totals.facts = totals.facts.saturating_add(1);
                totals.bytes = totals.bytes.saturating_add(name_size);
                for &id in row {
                    let size = crate::meter::stored_term_size(dict, id)?;
                    totals.largest = totals.largest.max(size);
                    totals.bytes = totals.bytes.saturating_add(size);
                }
            }
        }
    }
    totals.groups = index.len();
    totals.named = index.keys().filter(|key| key.is_some()).count();
    Ok((totals, index))
}

#[cfg(feature = "parallel")]
fn admit_parsed(
    totals: &ParsedTotals,
    complete: bool,
    ledger: &mut Ledger,
) -> Result<(), LoadError> {
    ledger.charge_term_batch(totals.bytes, totals.largest)?;
    ledger.charge(Dim::Facts, totals.facts)?;
    ledger.charge(
        Dim::Nodes,
        totals
            .facts
            .saturating_add(totals.groups)
            .saturating_add(if complete { totals.named } else { 0 }),
    )?;
    Ok(())
}

#[cfg(feature = "parallel")]
fn finish_parallel(
    text: &str,
    mut chunks: Vec<(usize, crate::nt::QuadChunk)>,
    ledger: &mut Ledger,
) -> Result<Graph, LoadError> {
    let error = chunks.iter().enumerate().find_map(|(i, (start, chunk))| {
        chunk.error.as_ref().map(|(offset, _)| (i, start + offset))
    });
    if let Some((i, _)) = error {
        chunks.truncate(i + 1);
    }
    let checkpoint = ledger.clone();
    let admitted = parsed_totals(&chunks).and_then(|(totals, index)| {
        admit_parsed(&totals, error.is_none(), ledger)?;
        Ok(index)
    });
    let mut index = match admitted {
        Ok(index) => index,
        Err(_) => {
            *ledger = checkpoint;
            drop(chunks);
            return load_serial(text, "nquads", None, true, ledger);
        }
    };
    if let Some((_, offset)) = error {
        // Only the incomplete statement and its suffix need the streaming parser.
        let offset = text.as_bytes()[..offset]
            .iter()
            .rposition(|b| matches!(b, b'\n' | b'\r'))
            .map_or(0, |i| i + 1);
        let (groups, complete_index) =
            load_serial_parts(text, offset, "nquads", None, true, ledger, index)?;
        index = complete_index;
        ledger.charge(Dim::Nodes, index.keys().filter(|key| key.is_some()).count())?;
        let buckets = groups
            .into_iter()
            .map(|(name, dict, rows)| {
                let key = match name {
                    Some(Term::NamedNode(n)) => Some(crate::nt::GraphKey::Iri(n.into_string())),
                    Some(Term::BlankNode(n)) => {
                        Some(crate::nt::GraphKey::Blank(n.into_string()))
                    }
                    _ => None,
                };
                (key, dict, rows)
            })
            .collect();
        chunks.push((
            offset,
            crate::nt::QuadChunk {
                buckets,
                error: None,
            },
        ));
    }
    drop(index);
    Ok(Graph::from_nquads_chunks(
        chunks.into_iter().map(|(_, chunk)| chunk.buckets),
    ))
}

#[cfg(all(test, feature = "parallel"))]
pub(crate) fn note_parallel_dataset_parse() {
    LOAD_PATHS.with(|paths| {
        let (serial, parallel) = paths.get();
        paths.set((serial, parallel + 1));
    });
}

#[cfg(test)]
std::thread_local! {
    static STREAMED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOAD_PATHS: std::cell::Cell<(usize,usize)> = const { std::cell::Cell::new((0,0)) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::Limits;

    fn snapshot(graph: &Graph) -> Vec<String> {
        let mut rows: Vec<_> = graph
            .iter_ids()
            .map(|row| format!("{:?}", row.map(|id| graph.dict().term(id))))
            .collect();
        for (name, part) in graph.named() {
            rows.extend(
                part.iter_ids()
                    .map(|row| format!("{:?} {name}", row.map(|id| part.dict().term(id)))),
            );
        }
        rows.sort();
        rows
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn dataset_loader_uses_the_existing_row_cutoff() {
        for (n, paths) in [(3071, (1, 0)), (3072, (0, 1))] {
            let source = (0..n)
                .map(|i| format!("<urn:s{i}> <urn:p> <urn:o> <urn:g> .\n"))
                .collect::<String>();
            LOAD_PATHS.with(|value| value.set((0, 0)));
            let mut bulk = Ledger::new(Limits::default());
            let graph = Graph::load_metered(&source, "nquads", &mut bulk).unwrap();
            LOAD_PATHS.with(|value| assert_eq!(value.get(), paths));
            let mut serial = Ledger::new(Limits::default());
            serial.charge(Dim::SourceBytes, source.len()).unwrap();
            serial.close_document();
            let reference = load_serial(&source, "nquads", None, true, &mut serial).unwrap();
            assert_eq!(snapshot(&graph), snapshot(&reference));
            for dim in [Dim::SourceBytes, Dim::Facts, Dim::Bytes, Dim::Nodes] {
                assert_eq!(bulk.spent(dim), serial.spent(dim), "{dim:?}");
            }
        }
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn source_refusal_precedes_parallel_dataset_parsing() {
        let source = (0..3200)
            .map(|i| format!("<urn:s{i}> <urn:p> <urn:o> <urn:g> .\n"))
            .collect::<String>();
        let limits = Limits {
            source_bytes: 0,
            ..Limits::default()
        };
        LOAD_PATHS.with(|paths| paths.set((0, 0)));
        let result = Graph::load_metered(&source, "nquads", &mut Ledger::new(limits));
        assert!(matches!(result,Err(LoadError::Ledger(error)) if error.dim==Dim::SourceBytes));
        LOAD_PATHS.with(|paths| assert_eq!(paths.get(), (0, 0)));
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_admission_keeps_streaming_boundaries_and_duplicates() {
        for duplicates in [false, true] {
            let source = (0..3200)
                .map(|i| {
                    format!(
                        "<urn:s{}> <urn:p> <urn:o> <urn:g> .\n",
                        if duplicates { i % 3 } else { i }
                    )
                })
                .collect::<String>();
            let mut reference = Ledger::new(Limits::default());
            reference.close_document();
            load_serial(&source, "nquads", None, true, &mut reference).unwrap();
            let mut unrestricted = Ledger::new(Limits::default());
            Graph::load_metered(&source, "nquads", &mut unrestricted).unwrap();
            for dim in [Dim::Facts, Dim::Bytes, Dim::Nodes] {
                assert_eq!(unrestricted.spent(dim), reference.spent(dim));
            }
            for dim in [Dim::Facts, Dim::Bytes, Dim::Nodes] {
                for extra in [0, 1] {
                    let cap = reference.spent(dim) - extra;
                    let mut limits = Limits::default();
                    match dim {
                        Dim::Facts => limits.facts = cap,
                        Dim::Bytes => limits.bytes = cap,
                        _ => limits.nodes = cap,
                    }
                    let mut actual = Ledger::new(limits);
                    let mut serial = Ledger::new(limits);
                    serial.charge(Dim::SourceBytes, source.len()).unwrap();
                    serial.close_document();
                    let expected = load_serial(&source, "nquads", None, true, &mut serial);
                    let got = Graph::load_metered(&source, "nquads", &mut actual);
                    assert_eq!(
                        got.as_ref().map(snapshot).map_err(ToString::to_string),
                        expected.as_ref().map(snapshot).map_err(ToString::to_string)
                    );
                    for dim in [Dim::SourceBytes, Dim::Facts, Dim::Bytes, Dim::Nodes] {
                        assert_eq!(actual.spent(dim), serial.spent(dim));
                    }
                }
            }
        }
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_occurrences_and_error_suffix_match_streaming_without_reparse() {
        for newline in ["\n", "\r\n", "\r"] {
            let prefix = (0..3200)
                .map(|i| {
                    format!(
                        "<urn:s{}> <urn:p> \"bonjour\"@fr <urn:g{}> .{newline}",
                        i % 11,
                        i % 3
                    )
                })
                .collect::<String>();
            for tail in [
                "",
                "invalid N-Quads",
                "<urn:s> <urn:p> \"unterminated",
                "<urn:s> <urn:p> <urn:o> . trailing",
                "  <urn:s> <urn:p> \"🦀\" <urn:g> !",
                "<urn:a> <urn:p> <urn:o> . <urn:b> <urn:p> <urn:o> .",
            ] {
                let source = format!("{prefix}{tail}");
                let mut actual = Ledger::new(Limits::default());
                actual.charge(Dim::Bytes, 7).unwrap();
                actual.charge(Dim::Facts, 2).unwrap();
                actual.charge(Dim::Nodes, 3).unwrap();
                let mut serial = actual.clone();
                serial.charge(Dim::SourceBytes, source.len()).unwrap();
                serial.close_document();
                let expected = load_serial(&source, "nquads", None, true, &mut serial);
                LOAD_PATHS.with(|paths| paths.set((0, 0)));
                STREAMED_BYTES.with(|bytes| bytes.set(0));
                let got = Graph::load_metered(&source, "nquads", &mut actual);
                assert_eq!(
                    got.as_ref().map(snapshot).map_err(ToString::to_string),
                    expected.as_ref().map(snapshot).map_err(ToString::to_string),
                    "{newline:?}: {tail}"
                );
                LOAD_PATHS.with(|paths| assert_eq!(paths.get(), (0, 1)));
                STREAMED_BYTES
                    .with(|bytes| assert!(bytes.get() <= tail.len(), "prefix was parsed again"));
                for dim in [
                    Dim::SourceBytes,
                    Dim::Bytes,
                    Dim::Facts,
                    Dim::Nodes,
                    Dim::DocumentBytes,
                ] {
                    assert_eq!(actual.spent(dim), serial.spent(dim), "{dim:?}: {tail}");
                }
            }
        }
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn native_only_directives_keep_one_streaming_pass() {
        let prefix = (0..3200)
            .map(|i| format!("<urn:s{i}> <urn:p> <urn:o> <urn:g> .\n"))
            .collect::<String>();
        let source = format!("{prefix}VERSION \"1.2\"\n");
        let mut serial = Ledger::new(Limits::default());
        serial.charge(Dim::SourceBytes, source.len()).unwrap();
        serial.close_document();
        let expected = load_serial(&source, "nquads", None, true, &mut serial)
            .err()
            .unwrap();
        LOAD_PATHS.with(|paths| paths.set((0, 0)));
        let mut actual = Ledger::new(Limits::default());
        let got = Graph::load_metered(&source, "nquads", &mut actual)
            .err()
            .unwrap();
        assert_eq!(got.to_string(), expected.to_string());
        LOAD_PATHS.with(|paths| assert_eq!(paths.get(), (1, 0)));
        for dim in [
            Dim::SourceBytes,
            Dim::Bytes,
            Dim::Facts,
            Dim::Nodes,
            Dim::DocumentBytes,
        ] {
            assert_eq!(actual.spent(dim), serial.spent(dim), "{dim:?}");
        }
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn bulk_refusal_after_precheck_restores_streaming_error_and_spent_work() {
        let source = (0..3200)
            .map(|i| format!("<urn:s{i}> <urn:p> <urn:o> <urn:g> .\n"))
            .collect::<String>();
        let mut actual = Ledger::new(Limits::default());
        actual.charge(Dim::GeneratedBytes, 17).unwrap();
        actual.charge(Dim::SourceBytes, source.len()).unwrap();
        actual.close_document();
        assert!(parallel_budget_covers(&source, &actual));
        let chunks = Graph::parse_nquads_chunks_report(source.as_bytes(), 4);
        // A later phase tightens a limit after the source pre-check.
        actual.cap_remaining(Limits {
            nodes: 2,
            ..Limits::default()
        });
        let (totals, _) = parsed_totals(&chunks).unwrap();
        let refused = admit_parsed(&totals, true, &mut actual.clone());
        assert!(matches!(refused, Err(LoadError::Ledger(error)) if error.dim == Dim::Nodes));
        let mut serial = actual.clone();
        let expected = load_serial(&source, "nquads", None, true, &mut serial)
            .err()
            .unwrap();
        let got = finish_parallel(&source, chunks, &mut actual).err().unwrap();
        assert_eq!(got.to_string(), expected.to_string());
        for dim in [
            Dim::SourceBytes,
            Dim::Bytes,
            Dim::DocumentBytes,
            Dim::TermBytes,
            Dim::Facts,
            Dim::Nodes,
            Dim::Depth,
            Dim::QueryRows,
            Dim::JoinRows,
            Dim::ClosureFacts,
            Dim::TermDepth,
            Dim::Alloc,
            Dim::Deadline,
            Dim::GeneratedBytes,
        ] {
            assert_eq!(actual.spent(dim), serial.spent(dim), "{dim:?}");
        }
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn bulk_term_sizes_and_depth_refusals_match_streaming() {
        let prefix = (0..3200)
            .map(|i| format!("<urn:s{i}> <urn:p> <urn:o> <urn:g> .\n"))
            .collect::<String>();
        for depth in [3, 65] {
            let mut object = "\"bonjour\"@fr".to_owned();
            for _ in 0..depth {
                object = format!("<<( <urn:a> <urn:b> {object} )>>");
            }
            let source=format!("{prefix}<urn:quoted> <urn:p> {object} <urn:g> .\n<urn:number> <urn:p> \"0\"^^<http://www.w3.org/2001/XMLSchema#integer> <urn:g> .\n");
            let mut actual = Ledger::new(Limits::default());
            let mut serial = Ledger::new(Limits::default());
            serial.charge(Dim::SourceBytes, source.len()).unwrap();
            serial.close_document();
            let expected = load_serial(&source, "nquads", None, true, &mut serial);
            let got = Graph::load_metered(&source, "nquads", &mut actual);
            assert_eq!(
                got.as_ref().map(snapshot).map_err(ToString::to_string),
                expected.as_ref().map(snapshot).map_err(ToString::to_string)
            );
            if depth == 65 {
                assert!(matches!(got,Err(LoadError::Ledger(error)) if error.dim==Dim::TermDepth));
            } else {
                assert!(got.is_ok());
            }
            for dim in [Dim::SourceBytes, Dim::Facts, Dim::Bytes, Dim::Nodes] {
                assert_eq!(actual.spent(dim), serial.spent(dim));
            }
        }
    }

    #[test]
    fn metered_load_preserves_dataset_and_folded_base_semantics() {
        for (source, format) in [
            (
                "<urn:s> <urn:p> <urn:o> <urn:g> .\n<urn:d> <urn:p> \"v\" .",
                "nquads",
            ),
            (
                "<urn:g> { <urn:s> <urn:p> <urn:o> } <urn:d> <urn:p> \"v\" .",
                "trig",
            ),
            ("<urn:s> <urn:p> \"v\" .", "ntriples"),
            ("@prefix e: <urn:> . e:s e:p \"v\" .", "turtle"),
        ] {
            let graph =
                Graph::load_metered(source, format, &mut Ledger::new(Limits::default())).unwrap();
            assert_eq!(
                snapshot(&graph),
                snapshot(&Graph::load_dataset(source, format).unwrap())
            );
            let mut projection_work = Ledger::new(Limits::default());
            let projection = graph.union_metered(&mut projection_work).unwrap();
            assert_eq!(projection_work.spent(crate::meter::Dim::SourceBytes), 0);
            assert_eq!(projection_work.spent(crate::meter::Dim::Facts), 0);
            let mut refused = Ledger::new(Limits { facts: 0, ..Limits::default() });
            assert!(graph.union_metered(&mut refused).is_ok());
            assert_eq!(
                snapshot(&projection),
                snapshot(&Graph::load_str(source, format).unwrap())
            );
            let graph = Graph::load_metered_with_base(
                source,
                format,
                "https://pod.ex/x",
                &mut Ledger::new(Limits::default()),
            )
            .unwrap();
            assert_eq!(
                snapshot(&graph),
                snapshot(&Graph::load_str_with_base(source, format, "https://pod.ex/x").unwrap())
            );
        }
        let graph = Graph::load_metered_with_base(
            "<#s> <urn:p> <relative> .",
            "ttl",
            "https://pod.ex/x",
            &mut Ledger::new(Limits::default()),
        )
        .unwrap();
        assert_eq!(
            snapshot(&graph),
            snapshot(
                &Graph::load_str_with_base("<#s> <urn:p> <relative> .", "ttl", "https://pod.ex/x")
                    .unwrap()
            )
        );
    }

    #[test]
    fn metered_load_refuses_source_and_facts_before_later_syntax() {
        for (source, bytes, facts, dimension) in [
            ("invalid", 0, usize::MAX, Dim::SourceBytes),
            (
                "<urn:s> <urn:p> <urn:o> .\ninvalid",
                usize::MAX,
                0,
                Dim::Facts,
            ),
        ] {
            let limits = Limits {
                source_bytes: bytes,
                facts,
                ..Limits::default()
            };
            let error = Graph::load_metered(source, "nquads", &mut Ledger::new(limits))
                .err()
                .unwrap();
            assert!(matches!(error, LoadError::Ledger(e) if e.dim == dimension));
        }
        let error = Graph::load_metered("invalid", "nquads", &mut Ledger::new(Limits::default()))
            .err()
            .unwrap();
        assert!(matches!(error, LoadError::Parse(_)));
    }
}
