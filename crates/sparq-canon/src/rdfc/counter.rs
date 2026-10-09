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
}

impl Default for SimpleHndqCallCounter {
    fn default() -> Self {
        Self {
            counter: Default::default(),
            limit: DEFAULT_HNDQ_CALL_LIMIT,
        }
    }
}

impl HndqCallCounter for SimpleHndqCallCounter {
    fn new(max_calls: Option<usize>) -> Self {
        let limit = match max_calls {
            Some(limit) => limit,
            None => DEFAULT_HNDQ_CALL_LIMIT,
        };
        Self { counter: 0, limit }
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
