//! Shared public layout for generated programs and independent reconstruction.
//! The authenticated profile binds its verifier-stored issuer root and slot limits.

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

/// Verifier-stored envelope policy in its public-input order.
/// Lane E derives these values from its typed policy; presentations never supply them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvelopeInputs {
    pub issuer_root: Word,
    pub issuer_depth: u32,
    pub holder_required: bool,
    pub holder_issuers_present: bool,
    pub holder_issuer_count: u32,
    pub holder_issuers: [Word; 4],
    pub status_required: bool,
    pub allow_missing_status: bool,
    pub snapshot_root: Word,
    pub snapshot_depth: u32,
    pub validity_required: bool,
    pub now: u64,
    pub freshness_required: bool,
    pub earliest_created: u64,
}
impl EnvelopeInputs {
    /// Existing issuer-membership profile, pending the canonical policy-root integration.
    pub fn issuer_only(issuer_root: Word) -> Self {
        Self {
            issuer_root,
            issuer_depth: crate::credential::ISSUER_DEPTH,
            holder_required: false,
            holder_issuers_present: false,
            holder_issuer_count: 0,
            holder_issuers: [[0; 32]; 4],
            status_required: false,
            allow_missing_status: false,
            snapshot_root: [0; 32],
            snapshot_depth: 0,
            validity_required: false,
            now: 0,
            freshness_required: false,
            earliest_created: 0,
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        field(self.issuer_root)?;
        field(self.snapshot_root)?;
        let n = self.holder_issuer_count as usize;
        if n > self.holder_issuers.len()
            || (!self.holder_required && self.holder_issuers_present)
            || (!self.holder_issuers_present && n != 0)
            || self.holder_issuers[n..].iter().any(|w| *w != [0; 32])
            || self.holder_issuers[..n]
                .iter()
                .any(|w| *w == [0; 32] || field(*w).is_err())
            || self.holder_issuers[..n].windows(2).any(|p| p[0] >= p[1])
            || (!self.status_required
                && (self.allow_missing_status
                    || self.snapshot_root != [0; 32]
                    || self.snapshot_depth != 0))
            || (!self.validity_required && self.now != 0)
            || (!self.freshness_required && self.earliest_created != 0)
        {
            return Err(Error::Invalid("noncanonical envelope policy".into()));
        }
        Ok(())
    }
    pub fn words(&self) -> Result<Vec<Word>, Error> {
        self.validate()?;
        let mut words = vec![
            self.issuer_root,
            number(u64::from(self.issuer_depth)),
            number(u64::from(self.holder_required)),
            number(u64::from(self.holder_issuers_present)),
            number(u64::from(self.holder_issuer_count)),
        ];
        words.extend(self.holder_issuers);
        words.extend([
            number(u64::from(self.status_required)),
            number(u64::from(self.allow_missing_status)),
            self.snapshot_root,
            number(u64::from(self.snapshot_depth)),
            number(u64::from(self.validity_required)),
            number(self.now),
            number(u64::from(self.freshness_required)),
            number(self.earliest_created),
        ]);
        Ok(words)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn require_consumer(&self) -> Result<(), Error> {
        if self.issuer_depth != crate::credential::ISSUER_DEPTH
            || self.holder_required
            || self.status_required
            || self.validity_required
            || self.freshness_required
        {
            return Err(Error::Unsupported(
                "credential envelope policy integration".into(),
            ));
        }
        Ok(())
    }
}

/// One source for public declarations, field flattening and ABI auditing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub rows: usize,
    pub credentials: usize,
    pub variables: usize,
    pub parameters: usize,
    pub indices: usize,
    pub form: ResultForm,
}

impl Layout {
    pub fn new(plan: &Plan) -> Self {
        Self {
            rows: plan.capacity().rows_per_table as usize,
            credentials: plan.capacity().credentials as usize,
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
    fn groups_authenticated(&self) -> Vec<(&'static str, Abi)> {
        let mut groups = self.groups();
        for group in Self::envelope_groups() {
            groups.insert(groups.len() - 1, group);
        }
        groups.insert(
            groups.len() - 1,
            (
                "credential_quad_bounds",
                Abi::array(Abi::UInt(32), self.credentials),
            ),
        );
        groups
    }
    fn envelope_groups() -> Vec<(&'static str, Abi)> {
        vec![
            ("issuer_root", Abi::Field),
            ("issuer_depth", Abi::UInt(32)),
            ("holder_required", Abi::Bool),
            ("holder_issuers_present", Abi::Bool),
            ("holder_issuer_count", Abi::UInt(32)),
            ("holder_issuers", Abi::array(Abi::Field, 4)),
            ("status_required", Abi::Bool),
            ("allow_missing_status", Abi::Bool),
            ("snapshot_root", Abi::Field),
            ("snapshot_depth", Abi::UInt(32)),
            ("validity_required", Abi::Bool),
            ("now", Abi::UInt(64)),
            ("freshness_required", Abi::Bool),
            ("earliest_created", Abi::UInt(64)),
        ]
    }
    pub(crate) fn envelope_arguments() -> String {
        Self::envelope_groups()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }
    pub fn declarations_authenticated(&self) -> String {
        self.groups_authenticated()
            .iter()
            .map(|(name, ty)| format!("    {name}: pub {},\n", ty.noir()))
            .collect()
    }
    pub fn reconstruct_authenticated(
        &self,
        plan: &Plan,
        statement: &Statement,
        issuer_root: Word,
    ) -> Result<Vec<Word>, Error> {
        self.reconstruct_authenticated_with_envelope(
            plan,
            statement,
            &EnvelopeInputs::issuer_only(issuer_root),
        )
    }
    pub fn reconstruct_authenticated_with_envelope(
        &self,
        plan: &Plan,
        statement: &Statement,
        envelope: &EnvelopeInputs,
    ) -> Result<Vec<Word>, Error> {
        let mut words =
            self.reconstruct_words(plan, statement, &crate::credential::SuiteEncoder, 1)?;
        words.extend(envelope.words()?);
        let mut bounds = plan.credential_quad_bounds();
        bounds.resize(self.credentials, 0);
        words.extend(bounds.into_iter().map(|n| number(u64::from(n))));
        words.push(binding(&words)?);
        Ok(words)
    }
    pub fn audit_authenticated_artifact(&self, artifact: &serde_json::Value) -> Result<(), Error> {
        self.audit_groups(artifact, self.groups_authenticated())
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
        let mut words = self.reconstruct_words(plan, statement, encoder, 0)?;
        words.push(binding(&words)?);
        Ok(words)
    }
    fn reconstruct_words(
        &self,
        plan: &Plan,
        statement: &Statement,
        encoder: &impl EncodeTerm,
        signature_mode: u8,
    ) -> Result<Vec<Word>, Error> {
        if *self != Self::new(plan) {
            return Err(Error::Invalid("public layout".into()));
        }
        if statement.signature_mode != signature_mode {
            return Err(Error::Unsupported(
                if signature_mode == 0 {
                    "credential suite opening"
                } else {
                    "hidden suite signature mode"
                }
                .into(),
            ));
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
        Ok(words)
    }
    /// Check the locally compiled ABI against the declaration schema.
    pub fn audit_artifact(&self, artifact: &serde_json::Value) -> Result<(), Error> {
        self.audit_groups(artifact, self.groups())
    }
    fn audit_groups(
        &self,
        artifact: &serde_json::Value,
        expected: Vec<(&str, Abi)>,
    ) -> Result<(), Error> {
        let parameters = artifact["abi"]["parameters"]
            .as_array()
            .ok_or_else(|| Error::Invalid("compiled ABI parameters".into()))?;
        let public: Vec<_> = parameters
            .iter()
            .filter(|p| p["visibility"] == "public")
            .collect();
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

#[cfg(test)]
mod literal_tests {
    use super::*;

    #[test]
    fn noir_expression_numeric_offset_matches_the_suite() {
        use crate::interp::EncodeTerm;
        let zero = oxrdf::Literal::new_typed_literal(
            "0",
            oxrdf::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        );
        let expected = crate::credential::SuiteEncoder
            .encode(&zero.into())
            .unwrap()
            .key;
        for source in [
            include_str!("../../../zk/noir-query/core/src/ops_core_expr.nr"),
            include_str!("../../../zk/noir-query/core/src/ops_core_tests.nr"),
        ] {
            let literal = source
                .split("let zero: Field = ")
                .nth(1)
                .unwrap()
                .split(';')
                .next()
                .unwrap();
            assert_eq!(
                field(expected).unwrap(),
                field_from_hex_str_strict(literal).unwrap()
            );
        }
    }

    #[test]
    fn noir_domains_are_literals_equal_to_the_host_domains() {
        let source = include_str!("../../../zk/noir-query/core/src/ops_core_statement.nr");
        let declarations: Vec<_> = source
            .lines()
            .filter(|line| line.starts_with("global "))
            .collect();
        assert_eq!(declarations.len(), 4);
        for (name, domain, bytes) in [
            ("STATEMENT_DOMAIN", STATEMENT_DOMAIN, *b"\0NQSTMT1"),
            ("CREDENTIAL_DOMAIN", CREDENTIAL_DOMAIN, *b"\0NQCELL1"),
        ] {
            assert_eq!(domain, u64::from_be_bytes(bytes));
            let declaration = declarations
                .iter()
                .find(|line| line.starts_with(&format!("global {name}:")))
                .unwrap();
            let literal = declaration.split_once('=').unwrap().1.trim();
            assert_eq!(
                u64::from_str_radix(
                    literal
                        .strip_suffix(';')
                        .unwrap()
                        .strip_prefix("0x")
                        .unwrap(),
                    16
                )
                .unwrap(),
                domain
            );
        }
        for (name, expected) in [
            ("DATASET_DOMAIN", DATASET_DOMAIN.as_slice()),
            (
                "SUITE_DATASET_DOMAIN",
                crate::credential::DATASET_DOMAIN.as_slice(),
            ),
        ] {
            let declaration = declarations
                .iter()
                .find(|line| line.starts_with(&format!("global {name}:")))
                .unwrap();
            let literal = declaration.split_once('=').unwrap().1.trim();
            let bytes: Vec<u8> = literal
                .strip_suffix(';')
                .unwrap()
                .strip_prefix('[')
                .unwrap()
                .strip_suffix(']')
                .unwrap()
                .split(',')
                .map(|b| b.trim().parse().unwrap())
                .collect();
            assert_eq!(bytes, expected);
        }
    }
}
