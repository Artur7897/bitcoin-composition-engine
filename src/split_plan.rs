use anyhow::{anyhow, bail, Result};
use std::collections::HashSet;

use crate::split_types::{SplitGroup, SplitPlanRequest, SplitPlanResponse};

pub fn run_split_plan(req: SplitPlanRequest) -> Result<SplitPlanResponse> {
    if req.input_utxo.trim().is_empty() {
        bail!("missing input UTXO");
    }

    if req.ordinals_address.trim().is_empty() {
        bail!("missing ordinals address");
    }

    if req.total_value == 0 {
        bail!("input UTXO has zero value");
    }

    let outputs = normalize_split_groups(req.groups, req.total_value)?;

    Ok(SplitPlanResponse {
        ok: true,
        splittable: true,
        output_count: outputs.len(),
        outputs,
    })
}

fn normalize_split_groups(groups: Vec<SplitGroup>, total_value: u64) -> Result<Vec<SplitGroup>> {
    let mut sorted = groups;
    sorted.sort_by_key(|group| group.offset);

    if sorted.len() < 2 {
        bail!("not composed");
    }

    if sorted.first().map(|group| group.offset) != Some(0) {
        bail!("first offset must be 0");
    }

    let mut seen_ids = HashSet::<String>::new();

    for group in &sorted {
        if group.ids.is_empty() {
            bail!("split group has no IDs");
        }

        if group.value == 0 {
            bail!("split group has zero postage");
        }

        for id in &group.ids {
            if id.trim().is_empty() {
                bail!("split group contains empty ID");
            }

            if !seen_ids.insert(id.clone()) {
                bail!("duplicate inscription ID: {}", id);
            }
        }
    }

    for pair in sorted.windows(2) {
        let expected_offset = pair[0]
            .offset
            .checked_add(pair[0].value)
            .ok_or_else(|| anyhow!("split boundary overflow"))?;

        if pair[1].offset != expected_offset {
            bail!(
                "invalid split geometry: expected offset {}, got {}",
                expected_offset,
                pair[1].offset
            );
        }
    }

    let last = sorted
        .last()
        .ok_or_else(|| anyhow!("split contains no groups"))?;

    let calculated_total = last
        .offset
        .checked_add(last.value)
        .ok_or_else(|| anyhow!("split total overflow"))?;

    if calculated_total != total_value {
        bail!(
            "split geometry does not match UTXO value: calculated {}, supplied {}",
            calculated_total,
            total_value
        );
    }

    Ok(sorted)
}
