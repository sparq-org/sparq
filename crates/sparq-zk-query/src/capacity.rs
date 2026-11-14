//! Public request bounds determine buckets; private cardinalities never select a key.

use crate::Error;

/// Verifier-selected opening bounds, compiled into the query program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpeningProfile {
    #[default]
    Solid,
    LongObjects,
    Extended,
}

/// Lexical backing limits for one opening profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpeningLimits {
    pub resource: u32,
    pub object: u32,
    pub scalar: u32,
    pub datatype: u32,
    pub language: u32,
    pub created: u32,
    pub method: u32,
}

impl OpeningProfile {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::LongObjects => "long-objects",
            Self::Extended => "extended",
        }
    }
    pub const fn limits(self) -> OpeningLimits {
        let (resource, object, method) = match self {
            Self::Solid => (64, 65, 32),
            Self::LongObjects => (64, 137, 32),
            Self::Extended => (128, 160, 128),
        };
        OpeningLimits {
            resource,
            object,
            scalar: 32,
            datatype: 41,
            language: 1,
            created: 32,
            method,
        }
    }
}

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
                &[4, 8, 16, 64],
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
