//! JSON-LD 1.1 Deserialize JSON-LD to RDF, Object to RDF, and List to RDF algorithms.

use crate::context::iri::{is_absolute_iri, is_blank_node};
use crate::from_rdf::{RdfQuad, RdfTerm};
use crate::{DocumentLoader, Json, JsonLdError, JsonLdErrorCode as E, JsonLdOptions, RdfDirection};
use std::collections::HashSet;

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// Expand a JSON-LD document (or a URL string loaded through `loader`) and convert it
/// to an RDF dataset. Honors `expand_context`, `rdf_direction`, and
/// `produce_generalized_rdf`. The returned dataset uses the crate-local RDF model.
pub fn to_rdf(
    input: &Json,
    options: &JsonLdOptions,
    loader: &dyn DocumentLoader,
) -> Result<Vec<RdfQuad>, JsonLdError> {
    let expanded = crate::expand(input, options, loader)?;
    let node_map = crate::node_map::try_generate_node_map(&expanded)?;
    let mut used = HashSet::new();
    for graph in node_map.graph_names() {
        used.insert(graph.to_string());
        if let Some(nodes) = node_map.graph(graph) {
            used.extend(nodes.subjects().map(str::to_string));
        }
    }
    let mut converter = Converter {
        options,
        used,
        next: 0,
        quads: Vec::new(),
    };
    for (graph, nodes) in node_map.into_graphs() {
        let graph = if graph == "@default" {
            None
        } else {
            let Some(name) = resource(&graph) else {
                continue;
            };
            Some(name)
        };
        for (id, node) in nodes {
            let Some(subject) = resource(&id) else {
                continue;
            };
            let Json::Obj(properties) = node else {
                continue;
            };
            for (property, values) in properties {
                let predicate = if property == "@type" {
                    RdfTerm::iri(format!("{RDF}type"))
                } else {
                    if property.starts_with('@')
                        || (is_blank_node(&property) && !options.produce_generalized_rdf)
                    {
                        continue;
                    }
                    let Some(p) = resource(&property) else {
                        continue;
                    };
                    p
                };
                let values = match values {
                    Json::Arr(a) => a,
                    other => vec![other],
                };
                for item in values {
                    let value = if property == "@type" {
                        item.as_str().and_then(resource)
                    } else {
                        converter.object(&item, &graph)?
                    };
                    if let Some(object) = value {
                        converter.quads.push(RdfQuad::new(
                            subject.clone(),
                            predicate.clone(),
                            object,
                            graph.clone(),
                        ));
                    }
                }
            }
        }
    }
    let mut seen = HashSet::new();
    converter.quads.retain(|q| seen.insert(q.clone()));
    Ok(converter.quads)
}

fn resource(id: &str) -> Option<RdfTerm> {
    if let Some(label) = id.strip_prefix("_:") {
        Some(RdfTerm::blank(label))
    } else if is_absolute_iri(id) {
        Some(RdfTerm::iri(id))
    } else {
        None
    }
}

struct Converter<'a> {
    options: &'a JsonLdOptions,
    used: HashSet<String>,
    next: usize,
    quads: Vec<RdfQuad>,
}

impl Converter<'_> {
    fn blank(&mut self) -> RdfTerm {
        loop {
            let label = format!("_:rdf{}", self.next);
            self.next += 1;
            if self.used.insert(label.clone()) {
                return RdfTerm::blank(&label[2..]);
            }
        }
    }

    fn object(
        &mut self,
        item: &Json,
        graph: &Option<RdfTerm>,
    ) -> Result<Option<RdfTerm>, JsonLdError> {
        if let Some(Json::Arr(items)) = item.get("@list") {
            let mut objects = Vec::new();
            for item in items {
                objects.push(self.object(item, graph)?);
            }
            let mut tail = RdfTerm::iri(format!("{RDF}nil"));
            for object in objects.into_iter().rev() {
                let cell = self.blank();
                if let Some(object) = object {
                    self.quads.push(RdfQuad::new(
                        cell.clone(),
                        RdfTerm::iri(format!("{RDF}first")),
                        object,
                        graph.clone(),
                    ));
                }
                self.quads.push(RdfQuad::new(
                    cell.clone(),
                    RdfTerm::iri(format!("{RDF}rest")),
                    tail,
                    graph.clone(),
                ));
                tail = cell;
            }
            return Ok(Some(tail));
        }
        let Some(value) = item.get("@value") else {
            return Ok(item.get("@id").and_then(Json::as_str).and_then(resource));
        };
        let datatype = item.get("@type").and_then(Json::as_str);
        if datatype.is_some_and(|d| d != "@json" && !is_absolute_iri(d)) {
            return Ok(None);
        }
        if datatype == Some("@json") {
            return Ok(Some(RdfTerm::typed_literal(
                canonical_json(value)?,
                format!("{RDF}JSON"),
            )));
        }
        if value.is_null() {
            return Ok(None);
        }
        let language = item.get("@language").and_then(Json::as_str);
        if language.is_some_and(|lang| !crate::from_rdf::well_formed_language(lang)) {
            return Ok(None);
        }
        let direction = item.get("@direction").and_then(Json::as_str);
        let lexical = match value {
            Json::Str(s) => s.clone(),
            Json::Raw(raw) if raw == "true" || raw == "false" => raw.clone(),
            Json::Raw(raw) => {
                let n: f64 = raw
                    .parse()
                    .map_err(|_| JsonLdError::new(E::InvalidValueObjectValue))?;
                if !n.is_finite() {
                    return Err(JsonLdError::new(E::InvalidValueObjectValue));
                }
                if datatype == Some("http://www.w3.org/2001/XMLSchema#double")
                    || n.fract() != 0.0
                    || n.abs() >= 1e21
                {
                    return Ok(Some(RdfTerm::typed_literal(
                        double(n),
                        datatype.unwrap_or(&format!("{XSD}double")),
                    )));
                }
                let lexical = if n == 0.0 {
                    "0".into()
                } else {
                    format!("{n:.0}")
                };
                return Ok(Some(RdfTerm::typed_literal(
                    lexical,
                    datatype.unwrap_or(&format!("{XSD}integer")),
                )));
            }
            _ => return Err(JsonLdError::new(E::InvalidValueObjectValue)),
        };
        if let Some(direction) = direction {
            match self.options.rdf_direction {
                RdfDirection::I18nDatatype => {
                    return Ok(Some(RdfTerm::typed_literal(
                        lexical,
                        format!(
                            "https://www.w3.org/ns/i18n#{}_{}",
                            language.unwrap_or("").to_lowercase(),
                            direction
                        ),
                    )))
                }
                RdfDirection::CompoundLiteral => {
                    let node = self.blank();
                    for (property, text) in [
                        ("value", Some(lexical.as_str())),
                        ("language", language),
                        ("direction", Some(direction)),
                    ] {
                        if let Some(text) = text {
                            self.quads.push(RdfQuad::new(
                                node.clone(),
                                RdfTerm::iri(format!("{RDF}{property}")),
                                RdfTerm::literal(text),
                                graph.clone(),
                            ));
                        }
                    }
                    return Ok(Some(node));
                }
                RdfDirection::None => {}
            }
        }
        Ok(Some(if let Some(language) = language {
            RdfTerm::lang_literal(lexical, language.to_lowercase())
        } else {
            let default_type = if matches!(value, Json::Raw(raw) if raw == "true" || raw == "false")
            {
                format!("{XSD}boolean")
            } else {
                format!("{XSD}string")
            };
            RdfTerm::typed_literal(lexical, datatype.unwrap_or(&default_type))
        }))
    }
}

fn double(number: f64) -> String {
    let scientific = format!("{number:.15E}");
    let (mantissa, exponent) = scientific.split_once('E').unwrap();
    let mantissa = mantissa.trim_end_matches('0');
    let mantissa = if mantissa.ends_with('.') {
        format!("{mantissa}0")
    } else {
        mantissa.into()
    };
    format!("{mantissa}E{}", exponent.parse::<i32>().unwrap())
}

fn canonical_json(value: &Json) -> Result<String, JsonLdError> {
    let mut out = String::new();
    match value {
        Json::Obj(members) => {
            let mut sorted: Vec<_> = members.iter().collect();
            sorted.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, (key, value)) in sorted.into_iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                Json::Str(key.clone()).write(&mut out);
                out.push(':');
                out.push_str(&canonical_json(value)?);
            }
            out.push('}');
        }
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                out.push_str(&canonical_json(item)?);
            }
            out.push(']');
        }
        Json::Raw(raw) if !["true", "false", "null"].contains(&raw.as_str()) => {
            let n: f64 = raw
                .parse()
                .map_err(|_| JsonLdError::new(E::InvalidJsonLiteral))?;
            if !n.is_finite() {
                return Err(JsonLdError::new(E::InvalidJsonLiteral));
            }
            if n == 0.0 {
                out.push('0');
            } else if n.abs() >= 1e21 || n.abs() < 1e-6 {
                let text = format!("{n:e}");
                let (m, e) = text.split_once('e').unwrap();
                let e: i32 = e.parse().unwrap();
                out.push_str(&format!("{m}e{}{e}", if e >= 0 { "+" } else { "" }));
            } else {
                out.push_str(&n.to_string());
            }
        }
        other => other.write(&mut out),
    }
    Ok(out)
}
