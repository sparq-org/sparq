//! Shared public layout for generated programs and independent reconstruction.
//! The supplied-cell opening is the programme's explicit draft stand-in.

use crate::{
    interp::{row_key, Cell, CredentialCells, EncodeTerm, ResultValue, Word},
    plan::{Parameter, Plan, ResultForm},
    Error,
};
use sha2::{Digest, Sha256};
use sparq_zk::{
    field::{field_from_hex_str_strict, field_to_be_bytes_32, Fr},
    poseidon2,
};

const STATEMENT_DOMAIN: u64 = 0x4e5153544d5431;
const CREDENTIAL_DOMAIN: u64 = 0x4e5143454c4c31;
const DATASET_DOMAIN: &[u8; 16] = b"NQ/SUPPLIED/V1\0\0";

/// Public statement values. Query structure, capacities and parameter values
/// are independently derived from the verifier's stored request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub request_digest: Word,
    pub input_commitment: Word,
    /// The draft supplied-cell profile uses absence of a signature mode.
    pub signature_mode: u8,
    pub result: ResultValue,
}

/// One source for public declarations, field flattening and ABI auditing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub rows: usize,
    pub variables: usize,
    pub parameters: usize,
    pub indices: usize,
    pub form: ResultForm,
}

impl Layout {
    pub fn new(plan: &Plan) -> Self {
        Self {
            rows: plan.capacity().rows_per_table as usize,
            variables: plan.result_schema().abi_width(),
            parameters: plan
                .parameters()
                .iter()
                .filter(|p| matches!(p, Parameter::Term(_)))
                .count()
                .max(1),
            indices: plan
                .parameters()
                .iter()
                .filter(|p| matches!(p, Parameter::Index(_)))
                .count()
                .max(1),
            form: plan.form(),
        }
    }
    fn groups(&self) -> Vec<(&'static str, Abi)> {
        vec![
            ("request_digest", Abi::array(Abi::UInt(8), 32)),
            ("input_commitment", Abi::array(Abi::UInt(8), 32)),
            ("signature_mode", Abi::UInt(8)),
            ("bounds", Abi::array(Abi::UInt(32), 3)),
            ("parameters", Abi::array(Abi::Cell, self.parameters)),
            ("indices", Abi::array(Abi::UInt(32), self.indices)),
            (
                "result",
                match self.form {
                    ResultForm::Ask => Abi::Bool,
                    ResultForm::Select => Abi::Table(self.rows, self.variables),
                },
            ),
            ("statement_binding", Abi::Field),
        ]
    }
    pub fn declarations(&self) -> String {
        self.groups()
            .iter()
            .map(|(name, ty)| format!("    {name}: pub {},\n", ty.noir()))
            .collect()
    }
    /// Reconstruct every public word without a presentation-supplied schema,
    /// plan, program, verification key or public-input blob.
    pub fn reconstruct(
        &self,
        plan: &Plan,
        statement: &Statement,
        encoder: &impl EncodeTerm,
    ) -> Result<Vec<Word>, Error> {
        if *self != Self::new(plan) {
            return Err(Error::Invalid("public layout".into()));
        }
        if statement.signature_mode != 0 {
            return Err(Error::Unsupported("credential suite opening".into()));
        }
        let mut words = vec![];
        words.extend(statement.request_digest.map(|b| number(u64::from(b))));
        words.extend(statement.input_commitment.map(|b| number(u64::from(b))));
        words.push(number(u64::from(statement.signature_mode)));
        let b = plan.bounds();
        words.extend(
            [b.credentials, b.quads_per_credential, b.rows_per_table].map(|v| number(u64::from(v))),
        );
        let mut parameters = vec![];
        let mut indices = vec![];
        for p in plan.parameters() {
            match p {
                Parameter::Term(t) => {
                    let c = encoder.encode(t)?;
                    c.validate()?;
                    if !c.bound || c.scope != 0 {
                        return Err(Error::Invalid("public constant".into()));
                    }
                    parameters.push(c);
                }
                Parameter::Index(i) => indices.push(*i),
            }
        }
        parameters.resize(self.parameters, Cell::default());
        indices.resize(self.indices, 0);
        for c in parameters {
            words.extend(cell_words(c));
        }
        words.extend(indices.into_iter().map(|i| number(u64::from(i))));
        match (&statement.result, plan.form()) {
            (ResultValue::Ask(value), ResultForm::Ask) => words.push(number(u64::from(*value))),
            (ResultValue::Select { variables, rows }, ResultForm::Select) => {
                let expected: Vec<_> = plan
                    .result_schema()
                    .variables
                    .iter()
                    .map(|&v| plan.variable_names()[v as usize].clone())
                    .collect();
                if *variables != expected {
                    return Err(Error::Invalid("result variables".into()));
                }
                if rows.len() > self.rows {
                    return Err(Error::Capacity("released rows".into()));
                }
                if rows.iter().any(|r| r.len() != expected.len()) {
                    return Err(Error::Invalid("result row width".into()));
                }
                if rows.windows(2).any(|rs| row_key(&rs[0]) > row_key(&rs[1])) {
                    return Err(Error::Invalid("result order".into()));
                }
                for i in 0..self.rows {
                    for j in 0..self.variables {
                        let c = rows
                            .get(i)
                            .and_then(|r| r.get(j))
                            .copied()
                            .unwrap_or_default();
                        c.validate()?;
                        if c.scope != 0 {
                            return Err(Error::Invalid("result-local blank node".into()));
                        }
                        words.extend(cell_words(c));
                    }
                }
                words.push(number(rows.len() as u64));
            }
            _ => return Err(Error::Invalid("result form".into())),
        }
        let binding = binding(&words)?;
        words.push(binding);
        Ok(words)
    }
    /// Check the locally compiled ABI against the declaration schema.
    pub fn audit_artifact(&self, artifact: &serde_json::Value) -> Result<(), Error> {
        let parameters = artifact["abi"]["parameters"]
            .as_array()
            .ok_or_else(|| Error::Invalid("compiled ABI parameters".into()))?;
        let public: Vec<_> = parameters
            .iter()
            .filter(|p| p["visibility"] == "public")
            .collect();
        let expected = self.groups();
        if public.len() != expected.len() {
            return Err(Error::Invalid("public-input ABI shape".into()));
        }
        for ((name, ty), actual) in expected.iter().zip(public) {
            if actual["name"] != *name || !ty.matches(&actual["type"]) {
                return Err(Error::Invalid(format!("public-input ABI {name}")));
            }
        }
        if !artifact["abi"]["return_type"].is_null() {
            return Err(Error::Invalid("public-input ABI return".into()));
        }
        Ok(())
    }
}

pub fn public_input_bytes(words: &[Word]) -> Vec<u8> {
    words.iter().flatten().copied().collect()
}
pub fn number(value: u64) -> Word {
    let mut w = [0; 32];
    w[24..].copy_from_slice(&value.to_be_bytes());
    w
}
pub fn hex(word: Word) -> String {
    format!(
        "0x{}",
        word.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}
pub(crate) fn field(word: Word) -> Result<Fr, Error> {
    field_from_hex_str_strict(&hex(word))
        .ok_or_else(|| Error::Invalid("noncanonical field word".into()))
}
pub(crate) fn cell_words(c: Cell) -> [Word; 8] {
    [
        number(u64::from(c.bound)),
        number(u64::from(c.kind)),
        c.enc,
        number(u64::from(c.key_class)),
        c.key,
        c.dt,
        c.lang,
        number(u64::from(c.scope)),
    ]
}
fn absorb(state: Fr, word: Word) -> Result<Fr, Error> {
    Ok(poseidon2::hash(&[state, field(word)?]))
}
pub fn binding(words: &[Word]) -> Result<Word, Error> {
    let mut state = Fr::from(STATEMENT_DOMAIN);
    for &word in words {
        state = absorb(state, word)?;
    }
    Ok(field_to_be_bytes_32(&state))
}

/// Salted commitment for the draft supplied-cell opening. The suite lane
/// replaces the private message derivation with its authenticated messages.
pub fn dataset_commitment(credentials: &[CredentialCells], salt: Word) -> Result<Word, Error> {
    let mut bytes = DATASET_DOMAIN.to_vec();
    bytes.extend(salt);
    for credential in credentials {
        let mut state = poseidon2::hash(&[
            Fr::from(CREDENTIAL_DOMAIN),
            Fr::from(credential.quads.len() as u64),
        ]);
        for quad in &credential.quads {
            for &c in quad {
                c.validate()?;
                for word in cell_words(c) {
                    state = absorb(state, word)?;
                }
            }
        }
        bytes.extend(field_to_be_bytes_32(&state));
    }
    Ok(Sha256::digest(bytes).into())
}

#[derive(Clone, Debug)]
enum Abi {
    UInt(u32),
    Bool,
    Field,
    Array(Box<Self>, usize),
    Cell,
    Table(usize, usize),
}
impl Abi {
    fn array(t: Self, n: usize) -> Self {
        Self::Array(Box::new(t), n)
    }
    fn noir(&self) -> String {
        match self {
            Self::UInt(n) => format!("u{n}"),
            Self::Bool => "bool".into(),
            Self::Field => "Field".into(),
            Self::Array(t, n) => format!("[{}; {n}]", t.noir()),
            Self::Cell => "TermCell".into(),
            Self::Table(r, v) => format!("Table<{r}, {v}>"),
        }
    }
    fn matches(&self, value: &serde_json::Value) -> bool {
        match self {
            Self::UInt(n) => {
                value["kind"] == "integer" && value["sign"] == "unsigned" && value["width"] == *n
            }
            Self::Bool => value["kind"] == "boolean",
            Self::Field => value["kind"] == "field",
            Self::Array(t, n) => {
                value["kind"] == "array" && value["length"] == *n && t.matches(&value["type"])
            }
            Self::Cell => structure(
                value,
                &[
                    ("bound", Self::Bool),
                    ("kind", Self::UInt(8)),
                    ("enc", Self::Field),
                    ("key_class", Self::UInt(8)),
                    ("key", Self::Field),
                    ("dt", Self::Field),
                    ("lang", Self::Field),
                    ("scope", Self::UInt(32)),
                ],
            ),
            Self::Table(r, v) => structure(
                value,
                &[
                    ("rows", Self::array(Self::array(Self::Cell, *v), *r)),
                    ("len", Self::UInt(32)),
                ],
            ),
        }
    }
}
fn structure(value: &serde_json::Value, expected: &[(&str, Abi)]) -> bool {
    value["kind"] == "struct"
        && value["fields"].as_array().is_some_and(|fields| {
            fields.len() == expected.len()
                && fields
                    .iter()
                    .zip(expected)
                    .all(|(f, (name, t))| f["name"] == *name && t.matches(&f["type"]))
        })
}
