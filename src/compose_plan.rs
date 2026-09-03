use anyhow::{anyhow, bail, Result};
use std::collections::HashSet;

use crate::compose_types::{ComposePlanRequest, ComposePlanResponse};

pub fn run_compose_plan(req: ComposePlanRequest) -> Result<ComposePlanResponse> {
    if req.root_id.trim().is_empty() {
        bail!("missing root ID");
    }

    if req.root_postage == 0 {
        bail!("root postage must be greater than zero");
    }

    if req.items.is_empty() {
        bail!("compose requires at least one item");
    }

    let mut seen_ids = HashSet::<String>::new();
    seen_ids.insert(req.root_id.clone());

    for item in &req.items {
        if item.id.trim().is_empty() {
            bail!("compose item contains empty ID");
        }

        if item.postage == 0 {
            bail!("compose item {} has zero postage", item.id);
        }

        if !seen_ids.insert(item.id.clone()) {
            bail!("duplicate compose inscription ID: {}", item.id);
        }
    }

    let mut planned_offsets = Vec::with_capacity(req.items.len());

    let mut next_offset = req.root_postage;

    for item in &req.items {
        planned_offsets.push(next_offset);

        next_offset = next_offset
            .checked_add(item.postage)
            .ok_or_else(|| anyhow!("compose offset overflow"))?;
    }

    Ok(ComposePlanResponse {
        ok: true,
        packable: true,
        planned_offsets,
    })
}
