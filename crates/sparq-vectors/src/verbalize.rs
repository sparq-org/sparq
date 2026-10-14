//! Entity **verbalization**: turning an entity's literal properties into the one text
//! string that gets embedded — the layer between the graph and the [`Embedder`].
//!
//! Production KG/vector systems embed a *passage* per entity, not a bare label:
//! Wikidata's vector database concatenates label + description + aliases + verbalized
//! statements; BLINK-style entity linking embeds `title [SEP] description`; Weaviate's
//! text2vec modules concatenate the schema's text properties with optional per-property
//! name prefixes (see `research/genai-text-embedding-practices.md` for sources). This
//! module is that convention for RDF:
//!
//! - [`EntityTextConfig`]: which predicates contribute, in **priority groups** (one
//!   value per group — label-like, type, description-like, extra prefixed literals), a
//!   **language preference chain** for choosing among multilingual literals, and a
//!   **character budget**.
//! - [`verbalize`]: the inspectable single-entity rendering,
//!   `(&Graph, &Term) -> Option<String>` — what [`embed_entities`] embeds, exposed so
//!   you can eyeball the texts before paying for a model.
//! - [`embed_entities`]: scans the graph through the predicate index blocks, verbalizes
//!   every entity that has at least one literal-group value, embeds the texts in
//!   batches, and `put`s each vector under the entity's dictionary id.
//!
//! [`embed_labels`](crate::embed_labels) is the label-only special case and now wraps
//! this module.
//!
//! ```
//! use sparq_core::Graph;
//! use sparq_vectors::{verbalize, EntityTextConfig};
//! # use oxrdf::{NamedNode, Term};
//!
//! let g = Graph::load_str(r#"
//!     @prefix rdfs:   <http://www.w3.org/2000/01/rdf-schema#> .
//!     @prefix schema: <http://schema.org/> .
//!     @prefix ex:     <http://example.org/> .
//!     ex:bolt rdfs:label "Usain Bolt"@en ;
//!             a ex:Athlete ;
//!             schema:description "Jamaican sprinter, eight-time Olympic champion."@en .
//!     ex:Athlete rdfs:label "athlete"@en .
//! "#, "turtle").unwrap();
//!
//! let cfg = EntityTextConfig::default();
//! let bolt = Term::NamedNode(NamedNode::new("http://example.org/bolt").unwrap());
//! assert_eq!(
//!     verbalize(&g, &bolt, &cfg).unwrap(),
//!     "Usain Bolt. a athlete. Jamaican sprinter, eight-time Olympic champion."
//! );
//! ```

use crate::embed::Embedder;
use crate::store::VectorStore;
use oxrdf::{NamedNode, Term};
use rustc_hash::FxHashSet;
use sparq_core::dict::{self, Id, TermParts};
use sparq_core::Graph;

/// How a [`PropertyGroup`] renders the objects it finds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ObjectKind {
    /// Take **literal** objects; the text is the literal's lexical value. Non-literal
    /// objects are skipped. Inline-integer literals are rendered too — only include a
    /// numeric predicate deliberately (and with a [`prefix`](PropertyGroup::prefix));
    /// raw numbers are usually better left to structured filters than embedded.
    #[default]
    Literal,
    /// Take **IRI / blank-node** objects and render each via its *own* label: the first
    /// literal found under the config's [`naming_predicates`](EntityTextConfig::naming_predicates)
    /// (honoring the language chain), falling back to the IRI's local name (the part
    /// after the last `#` or `/`). This is how `rdf:type` becomes the word "athlete"
    /// instead of a meaningless IRI. Literal objects are skipped.
    EntityLabel,
}

/// One slot of the verbalization template: a set of predicates tried in **priority
/// order** — the first predicate with at least one usable value supplies the group's
/// text, and later predicates in the group are ignored (so `rdfs:label` beats
/// `skos:prefLabel` for the same entity, exactly like
/// [`LabelConfig`](crate::LabelConfig)).
#[derive(Clone, Debug)]
pub struct PropertyGroup {
    /// Predicates in priority order within the group.
    pub predicates: Vec<NamedNode>,
    /// Prepended to the group's rendered value(s) — the Weaviate
    /// `vectorizePropertyName` convention. E.g. `"a "` for a type group
    /// (`…. a athlete.`) or `"occupation: "` for an extra literal.
    pub prefix: Option<String>,
    /// Literal values, or entity objects rendered via their labels.
    pub kind: ObjectKind,
    /// How many values the group may contribute (joined with `", "`). The winning
    /// predicate's candidates are ranked by the language chain, then by scan order.
    pub max_values: usize,
}

impl PropertyGroup {
    /// A [`ObjectKind::Literal`] group over `predicates`, one value, no prefix.
    pub fn literal(predicates: Vec<NamedNode>) -> PropertyGroup {
        PropertyGroup { predicates, prefix: None, kind: ObjectKind::Literal, max_values: 1 }
    }

    /// A [`ObjectKind::EntityLabel`] group over `predicates`, one value, no prefix.
    pub fn entity_label(predicates: Vec<NamedNode>) -> PropertyGroup {
        PropertyGroup { predicates, prefix: None, kind: ObjectKind::EntityLabel, max_values: 1 }
    }

    /// Sets the prefix (builder style).
    pub fn with_prefix(mut self, prefix: &str) -> PropertyGroup {
        self.prefix = Some(prefix.to_string());
        self
    }

    /// Sets the value cap (builder style).
    pub fn with_max_values(mut self, max_values: usize) -> PropertyGroup {
        self.max_values = max_values;
        self
    }
}

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).expect("static IRI")
}

/// The standard **label-like** predicates, in priority order: `rdfs:label`,
/// `skos:prefLabel`, `foaf:name`, `schema:name`, `dcterms:title`.
pub fn label_predicates() -> Vec<NamedNode> {
    vec![
        iri("http://www.w3.org/2000/01/rdf-schema#label"),
        iri("http://www.w3.org/2004/02/skos/core#prefLabel"),
        iri("http://xmlns.com/foaf/0.1/name"),
        iri("http://schema.org/name"),
        iri("http://purl.org/dc/terms/title"),
    ]
}

/// The standard **description-like** predicates, in priority order:
/// `schema:description`, `rdfs:comment`, `dcterms:description`, `skos:definition`,
/// `skos:note`.
pub fn description_predicates() -> Vec<NamedNode> {
    vec![
        iri("http://schema.org/description"),
        iri("http://www.w3.org/2000/01/rdf-schema#comment"),
        iri("http://purl.org/dc/terms/description"),
        iri("http://www.w3.org/2004/02/skos/core#definition"),
        iri("http://www.w3.org/2004/02/skos/core#note"),
    ]
}

/// Configuration for [`verbalize`] / [`embed_entities`]: the verbalization template.
///
/// The default renders the research-backed passage shape
/// **`<label>. a <type>. <description>`** (see
/// `research/genai-text-embedding-practices.md`):
///
/// 1. label group — [`label_predicates`];
/// 2. type group — `rdf:type` rendered via [`ObjectKind::EntityLabel`], prefix `"a "`;
/// 3. description group — [`description_predicates`].
///
/// Add further [`PropertyGroup`]s for domain literals worth embedding — short,
/// categorical, human-readable values, each with a prefix (`"occupation: "`). Leave
/// raw numbers and dates OUT (embedding models don't order numbers; keep those for
/// structured filters).
#[derive(Clone, Debug)]
pub struct EntityTextConfig {
    /// Template slots, in output order. Each contributes at most one rendered piece.
    pub groups: Vec<PropertyGroup>,
    /// Language preference chain, best first; matching is case-insensitive on the
    /// BCP-47 tag. `"en"` matches `@en`, `@en-GB` and the RDF 1.2 directional form
    /// stored as `en--ltr`; `""` matches plain (untagged) literals. Candidates in
    /// languages *not* in the chain rank after every listed language but are still
    /// used as a last resort (a graph labeled only in French still verbalizes).
    /// An empty chain means "no preference" (first value in scan order wins).
    /// Default: `["en", ""]`.
    pub languages: Vec<String>,
    /// Predicates used to find the label of an [`ObjectKind::EntityLabel`] object
    /// (e.g. the type IRI's own `rdfs:label`). Default: [`label_predicates`].
    pub naming_predicates: Vec<NamedNode>,
    /// Joins the groups' rendered pieces. Default: `". "`.
    pub separator: String,
    /// Character budget for the whole text. Pieces are appended in group order while
    /// they fit; the piece that overflows is truncated at a word or sentence boundary
    /// and ends the text. Default: 2048 —
    /// passage-sized, comfortably within every embedding model's window.
    pub max_chars: usize,
    /// Texts per [`Embedder::embed`] call in [`embed_entities`]. Default: 256.
    pub batch: usize,
}

impl Default for EntityTextConfig {
    fn default() -> Self {
        EntityTextConfig {
            groups: vec![
                PropertyGroup::literal(label_predicates()),
                PropertyGroup::entity_label(vec![iri(
                    "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
                )])
                .with_prefix("a "),
                PropertyGroup::literal(description_predicates()),
            ],
            languages: vec!["en".to_string(), String::new()],
            naming_predicates: label_predicates(),
            separator: ". ".to_string(),
            max_chars: 2048,
            batch: 256,
        }
    }
}

impl EntityTextConfig {
    /// A label-only template over `predicates` — what
    /// [`embed_labels_with`](crate::embed_labels_with) uses. No language preference
    /// (first label in scan order wins, preserving `embed_labels`' historical pick),
    /// no budget.
    pub fn labels_only(predicates: Vec<NamedNode>, batch: usize) -> EntityTextConfig {
        EntityTextConfig {
            groups: vec![PropertyGroup::literal(predicates)],
            languages: Vec::new(),
            naming_predicates: Vec::new(),
            separator: ". ".to_string(),
            max_chars: usize::MAX,
            batch,
        }
    }
}

/// Renders `entity`'s verbalization under `cfg`, or `None` when the entity has no
/// usable text. The exact string [`embed_entities`] would embed — inspect it before
/// paying for a model, or log it next to nearest-neighbour results.
///
/// Returns `Some` **iff at least one [`ObjectKind::Literal`] group contributed** —
/// entities whose only text would be a type word are skipped (a bare "a athlete"
/// passage matches every other athlete and nothing else). [`ObjectKind::EntityLabel`]
/// groups only ever *enrich* a literal-grounded text.
///
/// Cost: one `(s, p)`-bounded index range scan per configured predicate (plus one per
/// naming predicate for each `EntityLabel` object) — `O(groups · log n + degree)`.
pub fn verbalize(graph: &Graph, entity: &Term, cfg: &EntityTextConfig) -> Option<String> {
    let id = graph.id_of(entity)?;
    if dict::is_inline(id) {
        return None;
    }
    verbalize_id(graph, id, cfg)
}

/// [`verbalize`] for an already-resolved dictionary id (skips the term lookup).
fn verbalize_id(graph: &Graph, id: Id, cfg: &EntityTextConfig) -> Option<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut has_literal_text = false;
    for group in &cfg.groups {
        let Some(value) = group_value(graph, id, group, cfg) else { continue };
        has_literal_text |= group.kind == ObjectKind::Literal;
        match &group.prefix {
            Some(p) => pieces.push(format!("{p}{value}")),
            None => pieces.push(value),
        }
    }
    if !has_literal_text {
        return None;
    }
    // Assemble under the char budget: whole pieces while they fit; the overflowing
    // piece is truncated at a word or sentence boundary and ends the text.
    let mut out = String::new();
    for piece in pieces {
        let lead = if out.is_empty() { 0 } else { cfg.separator.chars().count() };
        let used = out.chars().count();
        let remaining = cfg.max_chars.saturating_sub(used + lead);
        if remaining == 0 {
            break;
        }
        if piece.chars().count() <= remaining {
            if !out.is_empty() {
                out.push_str(&cfg.separator);
            }
            out.push_str(&piece);
        } else {
            let truncated = truncate_at_word_boundary(&piece, remaining);
            if truncated.is_empty() {
                break;
            }
            if !out.is_empty() {
                out.push_str(&cfg.separator);
            }
            out.push_str(&truncated);
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Returns `s` unchanged when it fits; otherwise cuts at the last word or sentence
/// boundary within `max_chars`. The returned string never exceeds the character budget and never
/// ends partway through a whitespace-delimited token.
pub(crate) fn truncate_at_word_boundary(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }

    let mut chars = s.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    let next = chars.next();
    let last = prefix.chars().next_back();

    // A cut immediately before whitespace/punctuation, or immediately after sentence
    // punctuation, already lands on a boundary at exactly the requested budget.
    if next.is_some_and(is_boundary) || last.is_some_and(is_sentence_delimiter) {
        return prefix.trim_end().to_string();
    }

    for (index, ch) in prefix.char_indices().rev() {
        if ch.is_whitespace() {
            return prefix[..index].trim_end().to_string();
        }
        if is_sentence_delimiter(ch) {
            return prefix[..index + ch.len_utf8()].trim_end().to_string();
        }
    }
    String::new()
}

fn is_boundary(ch: char) -> bool {
    ch.is_whitespace() || is_sentence_delimiter(ch)
}

fn is_sentence_delimiter(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?')
}

/// The rendered value of one group for one entity: the first predicate (in group
/// order) with usable candidates wins; its candidates are ranked by language-chain
/// position then scan order, deduplicated, capped at `max_values`, joined with `", "`.
fn group_value(graph: &Graph, id: Id, group: &PropertyGroup, cfg: &EntityTextConfig) -> Option<String> {
    for pred in &group.predicates {
        let Some(pid) = graph.id_of(&Term::NamedNode(pred.clone())) else { continue };
        // (language rank, scan index, text) candidates from one contiguous SPO range.
        let mut cands: Vec<(usize, usize, String)> = Vec::new();
        let scan = graph.store.scan(&[Some(id), Some(pid), None]);
        for (i, row) in scan.rows.iter().enumerate() {
            let o = scan.to_spo(row)[2];
            let rendered = match group.kind {
                ObjectKind::Literal => literal_text(graph, o, &cfg.languages),
                ObjectKind::EntityLabel => entity_label_text(graph, o, cfg),
            };
            if let Some((rank, text)) = rendered {
                cands.push((rank, i, text));
            }
        }
        if cands.is_empty() {
            continue;
        }
        cands.sort_by_key(|a| (a.0, a.1));
        let mut values: Vec<String> = Vec::new();
        for (_, _, text) in cands {
            if values.len() >= group.max_values.max(1) {
                break;
            }
            if !values.contains(&text) {
                values.push(text);
            }
        }
        return Some(values.join(", "));
    }
    None
}

/// `id` as literal text: `Some((language rank, trimmed value))` for a non-empty
/// literal (inline integers decode to their lexical value, rank = unmatched), `None`
/// otherwise.
fn literal_text(graph: &Graph, id: Id, languages: &[String]) -> Option<(usize, String)> {
    if dict::is_inline(id) {
        // Inline xsd:integer — decoded via the dictionary, never language-tagged.
        let Term::Literal(l) = graph.dict.term(id) else { return None };
        return Some((lang_rank(languages, None), l.value().to_string()));
    }
    let TermParts::Lit { value, lang, .. } = graph.dict.term_parts(id) else { return None };
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some((lang_rank(languages, lang), value.to_string()))
}

/// `id` as an entity rendered by its own label: the best-language literal under the
/// config's naming predicates, else the IRI's local name / blank-node label. Rank 0 —
/// the *object's* language choice already happened; candidates compete on scan order.
fn entity_label_text(graph: &Graph, id: Id, cfg: &EntityTextConfig) -> Option<(usize, String)> {
    if dict::is_inline(id) {
        return None;
    }
    match graph.dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => {
            if let Some(label) = naming_label(graph, id, cfg) {
                return Some((0, label));
            }
            let full = format!("{prefix}{suffix}");
            let local = full.rsplit(['#', '/']).next().unwrap_or(&full);
            let local = if local.is_empty() { full.as_str() } else { local };
            Some((0, local.to_string()))
        }
        TermParts::Blank(_) => naming_label(graph, id, cfg).map(|l| (0, l)),
        _ => None,
    }
}

/// The best-language label of `id` under `cfg.naming_predicates`, if any.
fn naming_label(graph: &Graph, id: Id, cfg: &EntityTextConfig) -> Option<String> {
    let mut best: Option<(usize, usize, String)> = None;
    for pred in &cfg.naming_predicates {
        let Some(pid) = graph.id_of(&Term::NamedNode(pred.clone())) else { continue };
        let scan = graph.store.scan(&[Some(id), Some(pid), None]);
        for (i, row) in scan.rows.iter().enumerate() {
            let o = scan.to_spo(row)[2];
            if let Some((rank, text)) = literal_text(graph, o, &cfg.languages) {
                if best.as_ref().is_none_or(|(br, bi, _)| (rank, i) < (*br, *bi)) {
                    best = Some((rank, i, text));
                }
            }
        }
        if best.is_some() {
            return best.map(|(_, _, t)| t); // predicate priority: first with a label wins
        }
    }
    None
}

/// The position of `lang` in the preference chain, lower = better. `""` matches
/// untagged literals; a listed tag matches exactly or as a `-`-separated prefix
/// (`"en"` matches `en`, `en-gb`); the RDF 1.2 directional `lang--dir` storage
/// convention matches on the tag before `--`. Unmatched candidates rank
/// `languages.len()` — after everything listed, but still usable (fallback). An empty
/// chain ranks everything 0.
fn lang_rank(languages: &[String], lang: Option<&str>) -> usize {
    for (i, pref) in languages.iter().enumerate() {
        let matched = match (pref.is_empty(), lang) {
            (true, None) => true,
            (false, Some(tag)) => {
                // `en--ltr` → `en` (RDF 1.2 base direction, stored after `--`).
                let base = tag.split("--").next().unwrap_or(tag);
                base.eq_ignore_ascii_case(pref)
                    || (base.len() > pref.len()
                        && base.as_bytes()[pref.len()] == b'-'
                        && base[..pref.len()].eq_ignore_ascii_case(pref))
            }
            _ => false,
        };
        if matched {
            return i;
        }
    }
    languages.len()
}

/// Verbalizes and embeds every entity with at least one literal-group value under
/// `cfg`, writing one vector per entity into `store` (build phase). Returns the number
/// of entities embedded. The store is left **unfinalized** so callers can add more
/// vectors; call [`VectorStore::finalize`] when done.
///
/// Candidate entities are found by scanning each [`ObjectKind::Literal`] group
/// predicate's contiguous index block (no full-graph scan), exactly like
/// [`embed_labels`](crate::embed_labels); each candidate is then verbalized with
/// [`verbalize`] semantics — entities whose verbalization is `None` are skipped.
/// Deterministic: candidates keep first-seen scan order, and the embedder sees the
/// texts in that order, `cfg.batch` at a time.
///
/// ```no_run
/// use sparq_core::Graph;
/// use sparq_vectors::{embed_entities, EntityTextConfig, HashEmbedder, VectorStore};
///
/// # fn main() -> Result<(), String> {
/// # let g = Graph::load_str("", "turtle").map_err(|e| e.to_string())?;
/// let embedder = HashEmbedder::new(64); // test-only; bring your own Embedder
/// let mut store = VectorStore::create("graph.spqv", 64)?;
/// let n = embed_entities(&g, &mut store, &embedder, &EntityTextConfig::default())?;
/// store.finalize()?;
/// # Ok(()) }
/// ```
pub fn embed_entities(
    graph: &Graph,
    store: &mut VectorStore,
    embedder: &impl Embedder,
    cfg: &EntityTextConfig,
) -> Result<usize, String> {
    if embedder.dim() != store.dim() {
        return Err(format!(
            "embedder dim {} != store dim {}",
            embedder.dim(),
            store.dim()
        ));
    }
    // Candidates: subjects of any Literal-group predicate, first-seen order. Each
    // predicate block is one contiguous POS/PSO range — no full graph scan.
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut candidates: Vec<Id> = Vec::new();
    for group in &cfg.groups {
        if group.kind != ObjectKind::Literal {
            continue;
        }
        for pred in &group.predicates {
            let Some(pid) = graph.id_of(&Term::NamedNode(pred.clone())) else { continue };
            let scan = graph.store.scan(&[None, Some(pid), None]);
            for row in scan.rows.iter() {
                let [s, _, _] = scan.to_spo(row);
                if dict::is_inline(s) || seen.contains(&s) {
                    continue;
                }
                match graph.dict.term_parts(s) {
                    TermParts::Iri { .. } | TermParts::Blank(_) => {}
                    _ => continue, // only IRI / blank subjects are entities
                }
                seen.insert(s);
                candidates.push(s);
            }
        }
    }

    let verbalized: Vec<(Id, String)> = candidates
        .into_iter()
        .filter_map(|id| verbalize_id(graph, id, cfg).map(|text| (id, text)))
        .collect();

    for chunk in verbalized.chunks(cfg.batch.max(1)) {
        let texts: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        let vectors = embedder.embed(&texts)?;
        if vectors.len() != texts.len() {
            return Err(format!(
                "embedder returned {} vectors for {} texts",
                vectors.len(),
                texts.len()
            ));
        }
        for ((id, _), v) in chunk.iter().zip(vectors) {
            store.put(*id, &v)?;
        }
    }
    Ok(verbalized.len())
}

#[cfg(test)]
mod tests {
    use super::truncate_at_word_boundary;

    #[test]
    fn verbalize_truncation_uses_a_word_boundary_within_budget() {
        // A raw eight-character cut would produce "alpha be".
        let truncated = truncate_at_word_boundary("alpha beta gamma", 8);
        assert_eq!(truncated, "alpha");
        assert!(truncated.chars().count() <= 8);
        assert_eq!(truncate_at_word_boundary("alpha beta", 10), "alpha beta");
    }
}
