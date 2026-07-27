use anyhow::{anyhow, bail, Result};
use std::collections::HashSet;

use crate::compose_types::{ComposePlanRequest, ComposePlanResponse};
use crate::fees::{estimate_network_fee, service_fee_sats};

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

    let fee_rate = req.fee_rate.unwrap_or(1).max(1);

    let input_count = req
        .items
        .len()
        .checked_add(2)
        .ok_or_else(|| anyhow!("compose input count overflow"))?;

    let (_, network_fee) = estimate_network_fee(input_count, 3, fee_rate)?;

    let service_fee = service_fee_sats(req.payment_method.as_deref())?;

    let total = network_fee
        .checked_add(service_fee)
        .ok_or_else(|| anyhow!("total fee overflow"))?;

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
        network_fee,
        service_fee,
        total,
        planned_offsets,
    })
}
