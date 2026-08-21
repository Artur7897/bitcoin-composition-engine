use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExistingInsertGroup {
    pub ids: Vec<String>,
    pub offset: u64,
    pub postage: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct NewInsertGroup {
    pub ids: Vec<String>,
    pub input_utxo: String,
    pub postage: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InsertPlanRequest {
    pub input_utxo: String,
    pub ordinals_address: String,
    pub total_value: u64,

    /// Current UTXO state confirmed by Verify.
    pub existing_groups: Vec<ExistingInsertGroup>,

    /// Independent UTXOs to be inserted.
    pub insert_groups: Vec<NewInsertGroup>,

    /// Desired final order translated by Verify.
    /// BCE validates completeness and preserves the existing order.
    pub ordered_groups: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InsertPlanMode {
    DirectAppend,
    SplitAndInsert,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InsertGroupSource {
    Existing,
    Inserted,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedInsertGroup {
    pub source: InsertGroupSource,
    pub source_index: usize,
    pub ids: Vec<String>,
    pub offset: u64,
    pub postage: u64,
    pub input_utxo: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExistingRun {
    pub run_index: usize,
    pub ids: Vec<String>,
    pub groups: Vec<ExistingInsertGroup>,
    pub source_offset: u64,
    pub value: u64,

    /// For SplitAndInsert, the index corresponds to the parent vout.
    pub parent_output_index: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InsertChildInputKind {
    ExistingRun,
    Inserted,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsertChildInput {
    pub kind: InsertChildInputKind,
    pub value: u64,

    pub existing_run_index: Option<usize>,
    pub insert_index: Option<usize>,

    /// For DirectAppend: the existing composition UTXO.
    /// For Inserted: the new UTXO to be inserted.
    pub input_utxo: Option<String>,

    /// For SplitAndInsert: vout of the split parent.
    pub parent_output_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsertPlanResponse {
    pub ok: bool,
    pub insertable: bool,
    pub mode: InsertPlanMode,

    pub input_utxo: String,
    pub current_total_value: u64,
    pub final_total_value: u64,

    /// Contiguous regions of the existing composition.
    pub existing_runs: Vec<ExistingRun>,

    /// Required input order of the final insert transaction.
    pub child_inputs: Vec<InsertChildInput>,

    /// Final UTXO geometry recalculated by BCE.
    pub final_groups: Vec<PlannedInsertGroup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedSource {
    Existing(usize),
    Inserted(usize),
}

pub fn run_insert_plan(req: InsertPlanRequest) -> Result<InsertPlanResponse> {
    validate_request(&req)?;

    let mut existing_groups = req.existing_groups.clone();
    existing_groups.sort_by_key(|group| group.offset);

    validate_existing_geometry(&existing_groups, req.total_value)?;

    validate_insert_groups(&existing_groups, &req.insert_groups)?;

    let resolved_order = resolve_order(&existing_groups, &req.insert_groups, &req.ordered_groups)?;

    validate_resolved_order(
        &resolved_order,
        existing_groups.len(),
        req.insert_groups.len(),
    )?;

    let (mut existing_runs, existing_run_for_group) =
        build_existing_runs(&existing_groups, &resolved_order)?;

    let mode = if existing_runs.len() == 1 {
        InsertPlanMode::DirectAppend
    } else {
        InsertPlanMode::SplitAndInsert
    };

    if mode == InsertPlanMode::SplitAndInsert {
        for run in &mut existing_runs {
            run.parent_output_index =
                Some(u32::try_from(run.run_index).map_err(|_| anyhow!("too many existing runs"))?);
        }
    }

    let child_inputs = build_child_inputs(
        &req.input_utxo,
        &req.insert_groups,
        &existing_runs,
        &existing_run_for_group,
        &resolved_order,
        mode,
    )?;

    let final_groups = build_final_groups(&existing_groups, &req.insert_groups, &resolved_order)?;

    let inserted_total = req.insert_groups.iter().try_fold(0_u64, |total, group| {
        total
            .checked_add(group.postage)
            .ok_or_else(|| anyhow!("inserted value overflow"))
    })?;

    let final_total_value = req
        .total_value
        .checked_add(inserted_total)
        .ok_or_else(|| anyhow!("final value overflow"))?;

    let calculated_final_total = final_groups.iter().try_fold(0_u64, |total, group| {
        total
            .checked_add(group.postage)
            .ok_or_else(|| anyhow!("planned value overflow"))
    })?;

    if calculated_final_total != final_total_value {
        bail!(
            "final geometry value mismatch: calculated {}, expected {}",
            calculated_final_total,
            final_total_value
        );
    }

    Ok(InsertPlanResponse {
        ok: true,
        insertable: true,
        mode,
        input_utxo: req.input_utxo,
        current_total_value: req.total_value,
        final_total_value,
        existing_runs,
        child_inputs,
        final_groups,
    })
}

fn validate_request(req: &InsertPlanRequest) -> Result<()> {
    if req.input_utxo.trim().is_empty() {
        bail!("missing input UTXO");
    }

    if req.ordinals_address.trim().is_empty() {
        bail!("missing ordinals address");
    }

    if req.total_value == 0 {
        bail!("input UTXO has zero value");
    }

    if req.existing_groups.is_empty() {
        bail!("existing composition has no groups");
    }

    if req.insert_groups.is_empty() {
        bail!("missing insert groups");
    }

    if req.ordered_groups.is_empty() {
        bail!("missing ordered groups");
    }

    Ok(())
}

fn validate_existing_geometry(groups: &[ExistingInsertGroup], total_value: u64) -> Result<()> {
    let first = groups
        .first()
        .ok_or_else(|| anyhow!("existing composition has no groups"))?;

    if first.offset != 0 {
        bail!("first existing offset must be 0");
    }

    for group in groups {
        if group.ids.is_empty() {
            bail!("existing group has no IDs");
        }

        if group.postage == 0 {
            bail!("existing group at offset {} has zero postage", group.offset);
        }
    }

    for pair in groups.windows(2) {
        let expected_offset = pair[0]
            .offset
            .checked_add(pair[0].postage)
            .ok_or_else(|| anyhow!("existing boundary overflow"))?;

        if pair[1].offset != expected_offset {
            bail!(
                "invalid existing geometry: expected offset {}, got {}",
                expected_offset,
                pair[1].offset
            );
        }
    }

    let last = groups
        .last()
        .ok_or_else(|| anyhow!("existing composition has no groups"))?;

    let calculated_total = last
        .offset
        .checked_add(last.postage)
        .ok_or_else(|| anyhow!("existing total overflow"))?;

    if calculated_total != total_value {
        bail!(
            "existing geometry does not match UTXO value: calculated {}, supplied {}",
            calculated_total,
            total_value
        );
    }

    Ok(())
}

fn validate_insert_groups(
    existing_groups: &[ExistingInsertGroup],
    insert_groups: &[NewInsertGroup],
) -> Result<()> {
    let mut seen_ids = HashSet::<String>::new();
    let mut seen_utxos = HashSet::<String>::new();

    for group in existing_groups {
        for id in &group.ids {
            if id.trim().is_empty() {
                bail!("existing group contains empty ID");
            }

            if !seen_ids.insert(id.clone()) {
                bail!("duplicate inscription ID: {}", id);
            }
        }
    }

    for group in insert_groups {
        if group.ids.is_empty() {
            bail!("insert group has no IDs");
        }

        if group.input_utxo.trim().is_empty() {
            bail!("insert group has no input UTXO");
        }

        if group.postage == 0 {
            bail!("insert group has zero postage");
        }

        if !seen_utxos.insert(group.input_utxo.clone()) {
            bail!("insert UTXO selected more than once: {}", group.input_utxo);
        }

        for id in &group.ids {
            if id.trim().is_empty() {
                bail!("insert group contains empty ID");
            }

            if !seen_ids.insert(id.clone()) {
                bail!("duplicate inscription ID: {}", id);
            }
        }
    }

    Ok(())
}

fn resolve_order(
    existing_groups: &[ExistingInsertGroup],
    insert_groups: &[NewInsertGroup],
    ordered_groups: &[Vec<String>],
) -> Result<Vec<ResolvedSource>> {
    let expected_count = existing_groups
        .len()
        .checked_add(insert_groups.len())
        .ok_or_else(|| anyhow!("ordered group count overflow"))?;

    if ordered_groups.len() != expected_count {
        bail!(
            "ordered group count mismatch: expected {}, got {}",
            expected_count,
            ordered_groups.len()
        );
    }

    let mut resolved = Vec::with_capacity(ordered_groups.len());

    for ids in ordered_groups {
        if ids.is_empty() {
            bail!("ordered groups contain empty group");
        }

        let existing_match = existing_groups
            .iter()
            .position(|group| group.ids.as_slice() == ids.as_slice());

        let inserted_match = insert_groups
            .iter()
            .position(|group| group.ids.as_slice() == ids.as_slice());

        match (existing_match, inserted_match) {
            (Some(index), None) => {
                resolved.push(ResolvedSource::Existing(index));
            }

            (None, Some(index)) => {
                resolved.push(ResolvedSource::Inserted(index));
            }

            (None, None) => {
                bail!("ordered group not found: {:?}", ids);
            }

            (Some(_), Some(_)) => {
                bail!("ordered group is ambiguous: {:?}", ids);
            }
        }
    }

    Ok(resolved)
}

fn validate_resolved_order(
    resolved: &[ResolvedSource],
    existing_count: usize,
    inserted_count: usize,
) -> Result<()> {
    if resolved.first() != Some(&ResolvedSource::Existing(0)) {
        bail!("root group must remain first");
    }

    let existing_order: Vec<usize> = resolved
        .iter()
        .filter_map(|source| match source {
            ResolvedSource::Existing(index) => Some(*index),
            ResolvedSource::Inserted(_) => None,
        })
        .collect();

    let expected_existing_order: Vec<usize> = (0..existing_count).collect();

    if existing_order != expected_existing_order {
        bail!("insert must preserve existing group order");
    }

    let mut seen_inserted = HashSet::<usize>::new();

    for source in resolved {
        if let ResolvedSource::Inserted(index) = source {
            if !seen_inserted.insert(*index) {
                bail!("insert group appears more than once");
            }
        }
    }

    if seen_inserted.len() != inserted_count {
        bail!("not every insert group appears exactly once");
    }

    Ok(())
}

fn build_existing_runs(
    existing_groups: &[ExistingInsertGroup],
    resolved: &[ResolvedSource],
) -> Result<(Vec<ExistingRun>, Vec<usize>)> {
    let mut raw_runs = Vec::<Vec<ExistingInsertGroup>>::new();

    let mut run_for_group = vec![usize::MAX; existing_groups.len()];

    let mut previous_was_existing = false;

    for source in resolved {
        match source {
            ResolvedSource::Existing(index) => {
                if !previous_was_existing {
                    raw_runs.push(Vec::new());
                }

                let run_index = raw_runs
                    .len()
                    .checked_sub(1)
                    .ok_or_else(|| anyhow!("missing existing run"))?;

                raw_runs[run_index].push(existing_groups[*index].clone());

                run_for_group[*index] = run_index;
                previous_was_existing = true;
            }

            ResolvedSource::Inserted(_) => {
                previous_was_existing = false;
            }
        }
    }

    if run_for_group.contains(&usize::MAX) {
        bail!("existing group was not assigned to a run");
    }

    let mode_requires_parent = raw_runs.len() > 1;

    let mut runs = Vec::<ExistingRun>::new();

    for (run_index, groups) in raw_runs.into_iter().enumerate() {
        let source_offset = groups
            .first()
            .ok_or_else(|| anyhow!("empty existing run"))?
            .offset;

        let value = groups.iter().try_fold(0_u64, |total, group| {
            total
                .checked_add(group.postage)
                .ok_or_else(|| anyhow!("existing run overflow"))
        })?;

        let ids = groups
            .iter()
            .flat_map(|group| group.ids.iter().cloned())
            .collect();

        runs.push(ExistingRun {
            run_index,
            ids,
            groups,
            source_offset,
            value,
            parent_output_index: if mode_requires_parent {
                Some(u32::try_from(run_index).map_err(|_| anyhow!("run index overflow"))?)
            } else {
                None
            },
        });
    }

    Ok((runs, run_for_group))
}

fn build_child_inputs(
    input_utxo: &str,
    insert_groups: &[NewInsertGroup],
    existing_runs: &[ExistingRun],
    run_for_group: &[usize],
    resolved: &[ResolvedSource],
    mode: InsertPlanMode,
) -> Result<Vec<InsertChildInput>> {
    let mut child_inputs = Vec::<InsertChildInput>::new();

    let mut previous_run: Option<usize> = None;

    for source in resolved {
        match source {
            ResolvedSource::Existing(index) => {
                let run_index = run_for_group
                    .get(*index)
                    .copied()
                    .ok_or_else(|| anyhow!("missing run assignment"))?;

                if previous_run == Some(run_index) {
                    continue;
                }

                let run = existing_runs
                    .get(run_index)
                    .ok_or_else(|| anyhow!("missing existing run"))?;

                child_inputs.push(InsertChildInput {
                    kind: InsertChildInputKind::ExistingRun,
                    value: run.value,
                    existing_run_index: Some(run_index),
                    insert_index: None,
                    input_utxo: if mode == InsertPlanMode::DirectAppend {
                        Some(input_utxo.to_string())
                    } else {
                        None
                    },
                    parent_output_index: run.parent_output_index,
                });

                previous_run = Some(run_index);
            }

            ResolvedSource::Inserted(index) => {
                let group = insert_groups
                    .get(*index)
                    .ok_or_else(|| anyhow!("missing insert group"))?;

                child_inputs.push(InsertChildInput {
                    kind: InsertChildInputKind::Inserted,
                    value: group.postage,
                    existing_run_index: None,
                    insert_index: Some(*index),
                    input_utxo: Some(group.input_utxo.clone()),
                    parent_output_index: None,
                });

                previous_run = None;
            }
        }
    }

    Ok(child_inputs)
}

fn build_final_groups(
    existing_groups: &[ExistingInsertGroup],
    insert_groups: &[NewInsertGroup],
    resolved: &[ResolvedSource],
) -> Result<Vec<PlannedInsertGroup>> {
    let mut next_offset = 0_u64;
    let mut final_groups = Vec::<PlannedInsertGroup>::new();

    for source in resolved {
        let planned = match source {
            ResolvedSource::Existing(index) => {
                let group = existing_groups
                    .get(*index)
                    .ok_or_else(|| anyhow!("missing existing group"))?;

                PlannedInsertGroup {
                    source: InsertGroupSource::Existing,
                    source_index: *index,
                    ids: group.ids.clone(),
                    offset: next_offset,
                    postage: group.postage,
                    input_utxo: None,
                }
            }

            ResolvedSource::Inserted(index) => {
                let group = insert_groups
                    .get(*index)
                    .ok_or_else(|| anyhow!("missing inserted group"))?;

                PlannedInsertGroup {
                    source: InsertGroupSource::Inserted,
                    source_index: *index,
                    ids: group.ids.clone(),
                    offset: next_offset,
                    postage: group.postage,
                    input_utxo: Some(group.input_utxo.clone()),
                }
            }
        };

        next_offset = next_offset
            .checked_add(planned.postage)
            .ok_or_else(|| anyhow!("final offset overflow"))?;

        final_groups.push(planned);
    }

    Ok(final_groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn middle_insert_request() -> InsertPlanRequest {
        InsertPlanRequest {
            input_utxo: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0"
                .to_string(),
            ordinals_address: "bc1qznl7wxgtemt5eprmr6g3yj7nn7xh5gtzuvezuz".to_string(),
            total_value: 2646,
            existing_groups: vec![
                ExistingInsertGroup {
                    ids: ids(&["A"]),
                    offset: 0,
                    postage: 546,
                },
                ExistingInsertGroup {
                    ids: ids(&["B"]),
                    offset: 546,
                    postage: 700,
                },
                ExistingInsertGroup {
                    ids: ids(&["D"]),
                    offset: 1246,
                    postage: 600,
                },
                ExistingInsertGroup {
                    ids: ids(&["E"]),
                    offset: 1846,
                    postage: 800,
                },
            ],
            insert_groups: vec![NewInsertGroup {
                ids: ids(&["C"]),
                input_utxo: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:0"
                    .to_string(),
                postage: 650,
            }],
            ordered_groups: vec![
                ids(&["A"]),
                ids(&["B"]),
                ids(&["C"]),
                ids(&["D"]),
                ids(&["E"]),
            ],
        }
    }

    #[test]
    fn middle_insert_builds_split_parent() {
        let plan = run_insert_plan(middle_insert_request()).expect("middle insert must succeed");

        assert_eq!(plan.mode, InsertPlanMode::SplitAndInsert);

        assert_eq!(plan.existing_runs.len(), 2);
        assert_eq!(plan.existing_runs[0].value, 1246);
        assert_eq!(plan.existing_runs[1].value, 1400);

        assert_eq!(plan.existing_runs[0].parent_output_index, Some(0));

        assert_eq!(plan.existing_runs[1].parent_output_index, Some(1));

        assert_eq!(plan.child_inputs.len(), 3);

        assert_eq!(plan.child_inputs[0].kind, InsertChildInputKind::ExistingRun);

        assert_eq!(plan.child_inputs[1].kind, InsertChildInputKind::Inserted);

        assert_eq!(plan.child_inputs[2].kind, InsertChildInputKind::ExistingRun);

        assert_eq!(plan.final_groups[2].ids, ids(&["C"]));
        assert_eq!(plan.final_groups[2].offset, 1246);
        assert_eq!(plan.final_total_value, 3296);
    }

    #[test]
    fn append_insert_needs_no_split_parent() {
        let mut request = middle_insert_request();

        request.ordered_groups = vec![
            ids(&["A"]),
            ids(&["B"]),
            ids(&["D"]),
            ids(&["E"]),
            ids(&["C"]),
        ];

        let plan = run_insert_plan(request).expect("append insert must succeed");

        assert_eq!(plan.mode, InsertPlanMode::DirectAppend);

        assert_eq!(plan.existing_runs.len(), 1);

        assert_eq!(plan.existing_runs[0].parent_output_index, None);

        assert_eq!(
            plan.child_inputs[0].input_utxo.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0")
        );

        assert_eq!(plan.final_groups[4].offset, 2646);
    }

    #[test]
    fn multi_insert_builds_only_required_runs() {
        let mut request = middle_insert_request();

        request.existing_groups[3].ids = ids(&["F"]);

        request.insert_groups.push(NewInsertGroup {
            ids: ids(&["E"]),
            input_utxo: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee:0"
                .to_string(),
            postage: 550,
        });

        request.ordered_groups = vec![
            ids(&["A"]),
            ids(&["B"]),
            ids(&["C"]),
            ids(&["D"]),
            ids(&["E"]),
            ids(&["F"]),
        ];

        let plan = run_insert_plan(request).expect("multi-insert must succeed");

        assert_eq!(plan.mode, InsertPlanMode::SplitAndInsert);

        assert_eq!(plan.existing_runs.len(), 3);
        assert_eq!(plan.child_inputs.len(), 5);
        assert_eq!(plan.final_total_value, 3846);

        assert_eq!(
            plan.child_inputs
                .iter()
                .map(|input| input.kind)
                .collect::<Vec<_>>(),
            vec![
                InsertChildInputKind::ExistingRun,
                InsertChildInputKind::Inserted,
                InsertChildInputKind::ExistingRun,
                InsertChildInputKind::Inserted,
                InsertChildInputKind::ExistingRun,
            ]
        );
    }

    #[test]
    fn insertion_before_root_is_rejected() {
        let mut request = middle_insert_request();

        request.ordered_groups = vec![
            ids(&["C"]),
            ids(&["A"]),
            ids(&["B"]),
            ids(&["D"]),
            ids(&["E"]),
        ];

        let error = run_insert_plan(request).expect_err("root displacement must fail");

        assert!(error.to_string().contains("root group must remain first"));
    }

    #[test]
    fn reordering_existing_groups_is_rejected() {
        let mut request = middle_insert_request();

        request.ordered_groups = vec![
            ids(&["A"]),
            ids(&["D"]),
            ids(&["C"]),
            ids(&["B"]),
            ids(&["E"]),
        ];

        let error = run_insert_plan(request).expect_err("existing reorder must fail");

        assert!(error
            .to_string()
            .contains("insert must preserve existing group order"));
    }

    #[test]
    fn invalid_existing_geometry_is_rejected() {
        let mut request = middle_insert_request();

        request.existing_groups[1].offset = 547;
        request.existing_groups[1].postage = 699;

        let error = run_insert_plan(request).expect_err("invalid geometry must fail");

        assert!(error
            .to_string()
            .contains("invalid existing geometry: expected offset 546, got 547"));
    }
}
