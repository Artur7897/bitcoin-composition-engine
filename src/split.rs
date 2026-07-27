use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct SplitInput {
    pub input_utxo: String,
    pub ordinals_address: String,
    pub fee_rate: f64,
    pub total_value: u64,
    pub groups: Vec<SplitGroup>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SplitGroup {
    pub ids: Vec<String>,
    pub offset: u64,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SplitOutput {
    pub ids: Vec<String>,
    pub offset: u64,
    pub value: u64,
    pub address: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SplitPlan {
    pub input_utxo: String,
    pub ordinals_address: String,
    pub fee_rate: f64,
    pub outputs: Vec<SplitOutput>,
}

pub fn build_split_plan(input: SplitInput) -> Result<SplitPlan> {
    if input.input_utxo.trim().is_empty() {
        return Err(anyhow!("Missing Input Utxo"));
    }

    if input.ordinals_address.trim().is_empty() {
        return Err(anyhow!("Missing Ordinals Address"));
    }

    if input.groups.len() < 2 {
        return Err(anyhow!("Not Composed"));
    }

    let mut groups = input.groups.clone();
    groups.sort_by_key(|g| g.offset);

    if groups.first().map(|g| g.offset) != Some(0) {
        return Err(anyhow!("first offset must be 0"));
    }

    let mut outputs = Vec::new();

    for group in groups {
        if group.ids.is_empty() {
            return Err(anyhow!("split group has no ids"));
        }

        if group.value == 0 {
            return Err(anyhow!("invalid output value"));
        }

        outputs.push(SplitOutput {
            ids: group.ids,
            offset: group.offset,
            value: group.value,
            address: input.ordinals_address.clone(),
        });
    }

    Ok(SplitPlan {
        input_utxo: input.input_utxo,
        ordinals_address: input.ordinals_address,
        fee_rate: input.fee_rate,
        outputs,
    })
}