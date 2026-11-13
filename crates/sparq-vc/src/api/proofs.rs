use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{model::require, validate_proof, ApiError};

pub(super) struct Plan {
    pub order: Vec<usize>,
    pub dependencies: Vec<Vec<usize>>,
    pub referenced: Vec<bool>,
}

impl Plan {
    pub fn new(proofs: &[&Value], purpose: &str) -> Result<Self, ApiError> {
        require(!proofs.is_empty(), "proof must not be empty")?;
        require(proofs.len() <= 64, "proof count exceeds 64")?;
        let mut ids = BTreeMap::new();
        for (index, proof) in proofs.iter().enumerate() {
            validate_proof(proof, purpose)?;
            if let Some(id) = proof.get("id").and_then(Value::as_str) {
                require(ids.insert(id, index).is_none(), "duplicate proof id")?;
            }
        }
        let mut dependencies = vec![Vec::new(); proofs.len()];
        let mut referenced = vec![false; proofs.len()];
        for (index, proof) in proofs.iter().enumerate() {
            if let Some(previous) = proof.get("previousProof") {
                let previous: Vec<_> = previous
                    .as_array()
                    .map_or_else(|| vec![previous], |a| a.iter().collect());
                require(!previous.is_empty(), "previousProof must not be empty")?;
                require(previous.len() <= 64, "previousProof count exceeds 64")?;
                let mut seen = BTreeSet::new();
                for id in previous {
                    let id = id
                        .as_str()
                        .ok_or_else(|| ApiError::new("previousProof must contain strings"))?;
                    let &dependency = ids
                        .get(id)
                        .ok_or_else(|| ApiError::new("unknown previousProof id"))?;
                    require(dependency != index, "previousProof self-reference")?;
                    require(seen.insert(dependency), "duplicate previousProof reference")?;
                    dependencies[index].push(dependency);
                    referenced[dependency] = true;
                }
            }
        }
        let mut order = Vec::new();
        let mut state = vec![0; proofs.len()];
        let mut depths = vec![0; proofs.len()];
        for index in 0..proofs.len() {
            visit(index, &dependencies, &mut state, &mut depths, &mut order)?;
        }
        Ok(Self {
            order,
            dependencies,
            referenced,
        })
    }
}

fn visit(
    index: usize,
    dependencies: &[Vec<usize>],
    state: &mut [u8],
    depths: &mut [usize],
    order: &mut Vec<usize>,
) -> Result<usize, ApiError> {
    require(state[index] != 1, "cyclic previousProof references")?;
    if state[index] == 2 {
        return Ok(depths[index]);
    }
    state[index] = 1;
    let mut depth = 1;
    for &dependency in &dependencies[index] {
        depth = depth.max(1 + visit(dependency, dependencies, state, depths, order)?);
    }
    require(depth <= 16, "proof chain depth exceeds 16")?;
    depths[index] = depth;
    state[index] = 2;
    order.push(index);
    Ok(depth)
}
