use super::CanonicalizationError;
use std::fmt;

const DEFAULT_HNDQ_CALL_LIMIT: usize = 4000;

pub trait HndqCallCounter {
    fn new(max_calls: Option<usize>) -> Self;
    fn add(&mut self, identifier: &str) -> Result<(), CanonicalizationError>;
}

pub struct SimpleHndqCallCounter {
    counter: usize,
    limit: usize,
    permutations_left: Option<usize>,
}

impl Default for SimpleHndqCallCounter {
    fn default() -> Self {
        Self {
            counter: Default::default(),
            limit: DEFAULT_HNDQ_CALL_LIMIT,
            permutations_left: None,
        }
    }
}

impl HndqCallCounter for SimpleHndqCallCounter {
    fn new(max_calls: Option<usize>) -> Self {
        let limit = match max_calls {
            Some(limit) => limit,
            None => DEFAULT_HNDQ_CALL_LIMIT,
        };
        Self {
            counter: 0,
            limit,
            permutations_left: None,
        }
    }

    fn add(&mut self, _identifier: &str) -> Result<(), CanonicalizationError> {
        self.counter += 1;
        if self.counter > self.limit {
            Err(CanonicalizationError::HndqCallLimitExceeded(self.limit))
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for SimpleHndqCallCounter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("")
            .field("counter", &self.counter)
            .field("limit", &self.limit)
            .finish()
    }
}

impl SimpleHndqCallCounter {
    pub fn with_permutation_limit(mut self, limit: Option<usize>) -> Self {
        self.permutations_left = limit;
        self
    }

    pub fn permutation(&mut self) -> Result<(), CanonicalizationError> {
        if let Some(left) = self.permutations_left.as_mut() {
            *left = left
                .checked_sub(1)
                .ok_or(CanonicalizationError::PermutationLimitExceeded)?;
        }
        Ok(())
    }
}
