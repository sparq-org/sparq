//! JSON-LD 1.1 API HTML Content Algorithms, using HTML5 script-data tokenization.

mod dom;

use crate::context::iri::resolve_iri;
use crate::{Json, JsonLdError, JsonLdErrorCode as E};

#[derive(Default)]
struct Contents {
    base: Option<String>,
    scripts: Vec<(Option<String>, bool, String)>,
    current: Option<usize>,
    // The first matching id matters even if it is on a non-script element.
    ids: Vec<(String, Option<usize>)>,
}

pub(crate) fn extract(
    text: &str,
    url: &str,
    document_url: &str,
    all: bool,
) -> Result<(Json, String), JsonLdError> {
    extract_contents(dom::contents(text), url, document_url, all)
}

pub(crate) fn extract_xml(
    text: &str,
    url: &str,
    document_url: &str,
    all: bool,
) -> Result<(Json, String), JsonLdError> {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;
    let error = |e: String| JsonLdError::with_detail(E::LoadingDocumentFailed, e);
    let mut reader = quick_xml::NsReader::from_str(text);
    let mut doc = Contents::default();
    loop {
        let (namespace, event) = reader
            .read_resolved_event()
            .map_err(|e| error(e.to_string()))?;
        let html = matches!(namespace, ResolveResult::Unbound)
            || matches!(namespace, ResolveResult::Bound(ns) if ns.as_ref() == b"http://www.w3.org/1999/xhtml");
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                let mut id = None;
                let mut kind = None;
                let mut href = None;
                for attr in tag.attributes() {
                    let attr = attr.map_err(|e| error(e.to_string()))?;
                    let value = attr
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|e| error(e.to_string()))?
                        .into_owned();
                    match attr.key.as_ref() {
                        b"id" => id = Some(value),
                        b"type" => kind = Some(value),
                        b"href" => href = Some(value),
                        _ => {}
                    }
                }
                let script = html && tag.local_name().as_ref() == b"script";
                let index = script.then_some(doc.scripts.len());
                if let Some(id) = &id {
                    doc.ids.push((id.clone(), index));
                }
                if html && tag.local_name().as_ref() == b"base" && doc.base.is_none() {
                    doc.base = href;
                }
                if script {
                    let valid = kind.is_some_and(|t| {
                        t.split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .eq_ignore_ascii_case("application/ld+json")
                    });
                    doc.scripts.push((id, valid, String::new()));
                    if matches!(event, Event::Start(_)) {
                        doc.current = index;
                    }
                }
            }
            Event::End(tag) if html && tag.local_name().as_ref() == b"script" => doc.current = None,
            Event::Text(text) => {
                if let Some(index) = doc.current {
                    doc.scripts[index].2.push_str(
                        &text
                            .xml_content(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|e| error(e.to_string()))?,
                    );
                }
            }
            Event::CData(text) => {
                if let Some(index) = doc.current {
                    doc.scripts[index].2.push_str(
                        &text
                            .xml_content(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|e| error(e.to_string()))?,
                    );
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(index) = doc.current {
                    if let Some(ch) = reference
                        .resolve_char_ref()
                        .map_err(|e| error(e.to_string()))?
                    {
                        doc.scripts[index].2.push(ch);
                    } else {
                        let name = reference.decode().map_err(|e| error(e.to_string()))?;
                        let value = quick_xml::escape::resolve_xml_entity(&name)
                            .ok_or_else(|| error("unknown XML entity".into()))?;
                        doc.scripts[index].2.push_str(value);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    extract_contents(doc, url, document_url, all)
}

fn extract_contents(
    doc: Contents,
    url: &str,
    document_url: &str,
    all: bool,
) -> Result<(Json, String), JsonLdError> {
    let base = doc
        .base
        .as_deref()
        .map(|b| resolve_iri(document_url, b))
        .unwrap_or_else(|| document_url.to_string());
    let parse = |s: &str| {
        Json::parse(s).map_err(|e| JsonLdError::with_detail(E::InvalidScriptElement, e.to_string()))
    };
    if let Some((_, fragment)) = url.split_once('#') {
        let fragment = decode_fragment(fragment);
        let index = doc
            .ids
            .iter()
            .find(|(id, _)| *id == fragment)
            .and_then(|(_, i)| *i)
            .ok_or_else(|| JsonLdError::new(E::LoadingDocumentFailed))?;
        let (_, valid, text) = &doc.scripts[index];
        if !valid {
            return Err(JsonLdError::new(E::LoadingDocumentFailed));
        }
        return Ok((parse(text)?, base));
    }
    let mut scripts = doc.scripts.iter().filter(|(_, valid, _)| *valid);
    if !all {
        let script = scripts
            .next()
            .ok_or_else(|| JsonLdError::new(E::LoadingDocumentFailed))?;
        return Ok((parse(&script.2)?, base));
    }
    let mut result = Vec::new();
    for (_, _, text) in scripts {
        match parse(text)? {
            Json::Arr(items) => result.extend(items),
            value => result.push(value),
        }
    }
    Ok((Json::Arr(result), base))
}

fn decode_fragment(fragment: &str) -> String {
    let mut bytes = Vec::new();
    let mut source = fragment.as_bytes().iter().copied();
    while let Some(b) = source.next() {
        if b == b'%' {
            let mut look = source.clone();
            if let (Some(h), Some(l)) = (look.next(), look.next()) {
                if let (Some(h), Some(l)) = ((h as char).to_digit(16), (l as char).to_digit(16)) {
                    bytes.push((h * 16 + l) as u8);
                    source = look;
                    continue;
                }
            }
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
