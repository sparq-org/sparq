// [GPT-6] Bounded materialized wallet inputs for the common campaign runner.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Wallet {
    pub seed: String,
    pub scale: usize,
    pub credentials: Vec<Vec<Fact>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Fact {
    subject: String,
    predicate: String,
    text: Option<String>,
    integer: Option<u64>,
}

impl Wallet {
    pub fn generated(seed: &str, scale: usize) -> Fallible<Self> {
        if seed.len() != 16
            || !seed
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !(3..=128).contains(&scale)
        {
            return Err("wallet seed or candidate capacity rejected".into());
        }
        let mut names = Vec::new();
        let mut ages = Vec::new();
        for (person, name, age) in [
            ("alice", "Alice", 42),
            ("bob", "Bob", 12),
            ("carla", "Carla", 50),
        ] {
            let subject = format!("urn:sparq:bench:{seed}:{person}");
            names.push(Fact {
                subject: subject.clone(),
                predicate: "urn:name".into(),
                text: Some(name.into()),
                integer: None,
            });
            ages.push(Fact {
                subject,
                predicate: "urn:age".into(),
                text: None,
                integer: Some(age),
            });
        }
        let mut credentials = vec![names.clone(), ages.clone()];
        for index in 2..scale {
            let mut facts: Vec<_> = names.iter().chain(&ages).cloned().collect();
            // Each shared alternative has a distinct root, without changing successful query rows.
            facts.push(Fact {
                subject: format!("urn:sparq:bench:{seed}:credential:{index}"),
                predicate: "urn:padding".into(),
                text: Some("synthetic".into()),
                integer: None,
            });
            credentials.push(facts);
        }
        Ok(Self {
            seed: seed.into(),
            scale,
            credentials,
        })
    }

    pub fn validate(&self) -> Fallible<()> {
        if *self != Self::generated(&self.seed, self.scale)? {
            return Err("materialized wallet differs from the declared synthetic profile".into());
        }
        Ok(())
    }

    pub fn triples(&self) -> Fallible<Vec<Vec<Triple>>> {
        self.validate()?;
        self.credentials
            .iter()
            .map(|facts| {
                facts
                    .iter()
                    .map(|fact| {
                        let object = match (&fact.text, fact.integer) {
                            (Some(text), None) => Literal::new_simple_literal(text.clone()),
                            (None, Some(integer)) => Literal::new_typed_literal(
                                integer.to_string(),
                                NamedNode::new("http://www.w3.org/2001/XMLSchema#integer")?,
                            ),
                            _ => return Err("synthetic fact object rejected".into()),
                        };
                        Ok(Triple::new(
                            NamedNode::new(fact.subject.clone())?,
                            NamedNode::new(fact.predicate.clone())?,
                            object,
                        ))
                    })
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn materialized_wallet_is_exact_and_bounded() {
        for scale in [3, 8, 32, 128] {
            let wallet = Wallet::generated("0123456789abcdef", scale).unwrap();
            wallet.validate().unwrap();
            assert_eq!(wallet.triples().unwrap().len(), scale);
            let mut corrupt = wallet.clone();
            corrupt.credentials[0][0].text = Some("Mallory".into());
            assert!(corrupt.validate().is_err());
            let mut truncated = wallet;
            truncated.credentials.pop();
            assert!(truncated.validate().is_err());
        }
        assert!(Wallet::generated("0123456789abcdef", 129).is_err());
    }
}
