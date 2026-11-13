//! Shared fixed-point control; consumers own graph publication and rule ordering.

pub(crate) mod clock;

pub(crate) fn saturate(
    limit: usize,
    exhausted: &str,
    mut step: impl FnMut() -> Result<bool, String>,
) -> Result<usize, String> {
    for iteration in 1..=limit {
        if !step()? {
            return Ok(iteration);
        }
    }
    Err(exhausted.into())
}
