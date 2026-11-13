use std::collections::BTreeSet;

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::{DateTime, FixedOffset};
use oxrdf::NamedNode;
use serde_json::Value;

use super::{ApiError, OfflineContexts};

pub(super) const V2: &str = "https://www.w3.org/ns/credentials/v2";
pub(super) const V1: &str = "https://www.w3.org/2018/credentials/v1";

pub(super) fn require(ok: bool, message: &str) -> Result<(), ApiError> {
    if ok {
        Ok(())
    } else {
        Err(ApiError::new(message))
    }
}

pub(super) fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ApiError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::new(format!("{field} must be a string")))
}

fn iri(value: &Value) -> Result<(), ApiError> {
    require(
        value.as_str().is_some_and(|s| NamedNode::new(s).is_ok()),
        "identifier must be a single absolute IRI",
    )
}

pub(super) fn objects(value: &Value) -> Result<Vec<&Value>, ApiError> {
    let values = match value.as_array() {
        Some(values) => values.iter().collect::<Vec<_>>(),
        None => vec![value],
    };
    require(
        !values.is_empty() && values.iter().all(|v| v.is_object()),
        "expected one or more objects",
    )?;
    Ok(values)
}

fn types(value: &Value) -> Result<Vec<&str>, ApiError> {
    let values = value
        .as_array()
        .map_or_else(|| vec![value], |a| a.iter().collect());
    require(!values.is_empty(), "type must not be empty")?;
    values
        .into_iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ApiError::new("type must contain strings"))
        })
        .collect()
}

// The core proof validator implements XSD 1.1 calendar and timezone rules. Add the
// dateTimeStamp timezone requirement and use chrono only for temporal comparison.
pub(super) fn timestamp(value: &str) -> Result<DateTime<FixedOffset>, ApiError> {
    let config = crate::ProofConfig::new("urn:vc-api:timestamp").with_created(value);
    config
        .validate()
        .map_err(|_| ApiError::new("invalid dateTimeStamp"))?;
    DateTime::parse_from_rfc3339(value)
        .map_err(|_| ApiError::new("dateTimeStamp requires a timezone and a supported year"))
}

fn document(vc: &Value, cache: &OfflineContexts, kind: &str) -> Result<(), ApiError> {
    require(vc.is_object(), "document must be an object")?;
    let contexts = vc
        .get("@context")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::new("@context must be an ordered set"))?;
    require(
        matches!(contexts.first().and_then(Value::as_str), Some(V2 | V1)),
        "first context must be the VC v2 or v1 context",
    )?;
    let mut urls = BTreeSet::new();
    for context in contexts {
        require(
            context.is_object() || context.is_string(),
            "invalid @context item",
        )?;
        if let Some(url) = context.as_str() {
            require(urls.insert(url), "duplicate context URL")?;
            cache.bytes(url)?;
        }
    }
    require(
        types(&vc["type"])?.contains(&kind),
        &format!("type must include {kind}"),
    )?;
    if let Some(id) = vc.get("id") {
        iri(id)?;
    }
    Ok(())
}

pub(super) fn presentation(vp: &Value, cache: &OfflineContexts) -> Result<(), ApiError> {
    document(vp, cache, "VerifiablePresentation")?;
    if let Some(holder) = vp.get("holder") {
        iri(if holder.is_object() {
            &holder["id"]
        } else {
            holder
        })?;
    }
    if let Some(credentials) = vp.get("verifiableCredential") {
        require(
            objects(credentials)?.len() <= 32,
            "presentation exceeds 32 credentials",
        )?;
    }
    Ok(())
}

pub(super) fn validate(vc: &Value, cache: &OfflineContexts) -> Result<(), ApiError> {
    document(vc, cache, "VerifiableCredential")?;
    let contexts = vc["@context"].as_array().expect("validated contexts");
    let issuer = vc
        .get("issuer")
        .ok_or_else(|| ApiError::new("issuer is required"))?;
    if issuer.is_object() {
        iri(&issuer["id"])?;
    } else {
        iri(issuer)?;
    }
    let subjects = objects(
        vc.get("credentialSubject")
            .ok_or_else(|| ApiError::new("credentialSubject is required"))?,
    )?;
    for subject in subjects {
        require(
            subject.as_object().is_some_and(|v| !v.is_empty()),
            "credentialSubject must contain a claim",
        )?;
        if let Some(id) = subject.get("id") {
            iri(id)?;
        }
    }
    for field in [
        "credentialStatus",
        "termsOfUse",
        "evidence",
        "refreshService",
        "credentialSchema",
    ] {
        if let Some(value) = vc.get(field) {
            for item in objects(value)? {
                types(&item["type"])?;
                if let Some(id) = item.get("id") {
                    iri(id)?;
                }
                if field == "credentialSchema" {
                    iri(&item["id"])?;
                }
            }
        }
    }
    for node in [vc, issuer] {
        for field in ["name", "description"] {
            if let Some(value) = node.get(field) {
                language_values(value)?;
            }
        }
    }
    let mut dates = Vec::new();
    for field in if contexts[0] == V2 {
        ["validFrom", "validUntil"]
    } else {
        ["issuanceDate", "expirationDate"]
    } {
        dates.push(
            vc.get(field)
                .map(|_| string(vc, field).and_then(timestamp))
                .transpose()?,
        );
    }
    if let (Some(from), Some(until)) = (dates[0], dates[1]) {
        require(from <= until, "validUntil precedes validFrom")?;
    }
    if let Some(resources) = vc.get("relatedResource") {
        let mut ids = BTreeSet::new();
        for resource in objects(resources)? {
            iri(&resource["id"])?;
            let id = string(resource, "id")?;
            require(ids.insert(id), "duplicate relatedResource id")?;
            require(
                resource.get("digestSRI").is_some() || resource.get("digestMultibase").is_some(),
                "relatedResource requires a digest",
            )?;
            if let Some(value) = resource.get("mediaType") {
                require(
                    value.as_str().is_some_and(|s| !s.is_empty()),
                    "relatedResource mediaType must be a string",
                )?;
            }
            if let Some(value) = resource.get("digestSRI") {
                let valid = value.as_str().is_some_and(|s| {
                    s.split_once('-').is_some_and(|(algorithm, digest)| {
                        let length = match algorithm {
                            "sha256" => 32,
                            "sha384" => 48,
                            "sha512" => 64,
                            _ => return false,
                        };
                        STANDARD
                            .decode(digest)
                            .is_ok_and(|bytes| bytes.len() == length)
                    })
                });
                require(valid, "invalid relatedResource digestSRI")?;
            }
            if let Some(value) = resource.get("digestMultibase") {
                let valid = value.as_str().is_some_and(|s| match s.strip_prefix('u') {
                    Some(encoded) => URL_SAFE_NO_PAD
                        .decode(encoded)
                        .is_ok_and(|bytes| bytes.len() == 32),
                    None => false,
                });
                require(valid, "invalid relatedResource digestMultibase")?;
            }
        }
    }
    Ok(())
}

fn language_values(value: &Value) -> Result<(), ApiError> {
    let values = value
        .as_array()
        .map_or_else(|| vec![value], |a| a.iter().collect());
    require(!values.is_empty(), "language value set must not be empty")?;
    for value in values {
        if value.is_string() {
            continue;
        }
        let object = value
            .as_object()
            .ok_or_else(|| ApiError::new("invalid language value"))?;
        require(
            object
                .keys()
                .all(|k| matches!(k.as_str(), "@value" | "@language" | "@direction"))
                && value["@value"].is_string(),
            "invalid language value object",
        )?;
        if let Some(language) = value.get("@language") {
            require(
                language.as_str().is_some_and(|s| !s.is_empty()),
                "invalid language",
            )?;
        }
        if let Some(direction) = value.get("@direction") {
            require(
                matches!(direction.as_str(), Some("ltr" | "rtl")),
                "invalid direction",
            )?;
        }
    }
    Ok(())
}

pub(super) fn expanded_types(value: &Value) -> Result<(), ApiError> {
    match value {
        Value::Object(object) => {
            if let Some(value) = object.get("@type") {
                for term in types(value)? {
                    iri(&Value::String(term.into()))?;
                }
            }
            for child in object.values() {
                expanded_types(child)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                expanded_types(child)?;
            }
        }
        _ => (),
    }
    Ok(())
}
