//! Public request bounds determine buckets; private cardinalities never select a key.

use crate::Error;

/// Request limits. They apply before rounding to a compiled capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub credentials: u32,
    pub quads_per_credential: u32,
    pub rows_per_table: u32,
}

/// Compiled capacities, revealed by the verification key and method parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub credentials: u32,
    pub quads_per_credential: u32,
    pub rows_per_table: u32,
}

fn bucket(value: u32, choices: &[u32], name: &str) -> Result<u32, Error> {
    choices
        .iter()
        .copied()
        .find(|&n| value > 0 && value <= n)
        .ok_or_else(|| Error::Capacity(name.into()))
}

impl Bounds {
    /// Choose the smallest bucket admitting each public request bound.
    pub fn capacity(self) -> Result<Capacity, Error> {
        Ok(Capacity {
            credentials: bucket(self.credentials, &[1, 2, 4], "credentials")?,
            quads_per_credential: bucket(
                self.quads_per_credential,
                &[4, 16, 64, 256],
                "quads per credential",
            )?,
            rows_per_table: bucket(self.rows_per_table, &[2, 4, 8, 16, 32], "rows per table")?,
        })
    }

    /// Check private counts against the request, without changing its bucket.
    pub fn check_credentials(self, quad_counts: &[usize]) -> Result<(), Error> {
        self.capacity()?;
        if quad_counts.len() > self.credentials as usize
            || quad_counts
                .iter()
                .any(|&n| n > self.quads_per_credential as usize)
        {
            return Err(Error::Capacity("credential dataset".into()));
        }
        Ok(())
    }
}
