use crate::codec::{bytes, err, hex, word, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sparq_zk_query::{answer::Answer, method::Presentation};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ask {
    pub boolean: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Select {
    pub variables: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum AnswerWire {
    Ask(Ask),
    Select(Select),
}
impl AnswerWire {
    pub fn from_answer(answer: &Answer) -> Self {
        match answer {
            Answer::Ask(boolean) => Self::Ask(Ask { boolean: *boolean }),
            Answer::Select { variables, rows } => Self::Select(Select {
                variables: variables.clone(),
                rows: rows
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|t| t.as_ref().map(ToString::to_string))
                            .collect()
                    })
                    .collect(),
            }),
        }
    }
    pub fn answer(&self) -> Result<Answer> {
        Ok(match self {
            Self::Ask(a) => Answer::Ask(a.boolean),
            Self::Select(a) => Answer::Select {
                variables: a.variables.clone(),
                rows: a
                    .rows
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|t| t.as_deref().map(str::parse).transpose().map_err(err))
                            .collect()
                    })
                    .collect::<Result<_>>()?,
            },
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PresentationWire {
    pub request_digest: String,
    pub input_commitment: String,
    pub signature_mode: u8,
    pub result: AnswerWire,
    pub proof: String,
}
impl PresentationWire {
    pub fn native(&self) -> Result<Presentation> {
        Ok(Presentation {
            request_digest: word(&self.request_digest)?,
            input_commitment: word(&self.input_commitment)?,
            signature_mode: self.signature_mode,
            result: self.result.answer()?,
            proof: bytes(&self.proof)?,
        })
    }
    pub fn from_native(p: &Presentation) -> Self {
        Self {
            request_digest: hex(&p.request_digest),
            input_commitment: hex(&p.input_commitment),
            signature_mode: p.signature_mode,
            result: AnswerWire::from_answer(&p.result),
            proof: hex(&p.proof),
        }
    }
}
pub(crate) fn refusal(reason: impl ToString) -> Value {
    json!({"verified":false,"released":[],"reason":reason.to_string()})
}
