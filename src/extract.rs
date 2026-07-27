use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExtractGroup {
    pub ids: Vec<String>,
    pub offset: u64,
    pub postage: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractPlanRequest {
    pub input_utxo: String,
    pub ordinals_address: String,
    pub total_value: u64,

    /// Vollständiger, von Verify übersetzter UTXO-Zustand.
    pub groups: Vec<ExtractGroup>,

    /// Eine oder mehrere Gruppen, die jeweils als eigene UTXO
    /// extrahiert werden sollen.
    pub extract_groups: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtractOutputKind {
    Remainder,
    Extracted,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractOutput {
    pub kind: ExtractOutputKind,
    pub ids: Vec<String>,

    /// Offset innerhalb der ursprünglichen Input-UTXO.
    pub source_offset: u64,

    /// Tatsächlicher Wert des Parent-Outputs.
    pub value: u64,

    pub address: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractedOutputRef {
    pub ids: Vec<String>,
    pub source_offset: u64,
    pub postage: u64,
    pub output_index: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecomposeIntent {
    /// Vout-Indizes aller Restbereiche der Parent-Transaktion.
    pub input_output_indices: Vec<u32>,

    /// Vom Core neu normalisierte Rest-Composition.
    pub groups: Vec<ExtractGroup>,

    pub total_value: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractPlanResponse {
    pub ok: bool,
    pub extractable: bool,
    pub input_utxo: String,

    /// Parent-Outputs in zwingender Sat-Reihenfolge.
    pub outputs: Vec<ExtractOutput>,

    /// Konkrete Vout-Position jedes extrahierten Objekts.
    pub extracted_outputs: Vec<ExtractedOutputRef>,

    /// Nur vorhanden, wenn mehrere Restbereiche verbunden werden müssen.
    pub recompose: Option<RecomposeIntent>,
}

pub fn run_extract_plan(req: ExtractPlanRequest) -> Result<ExtractPlanResponse> {
    validate_request(&req)?;

    let mut groups = req.groups.clone();
    groups.sort_by_key(|group| group.offset);

    validate_geometry(&groups, req.total_value)?;

    let selected_indices = resolve_extract_groups(&groups, &req.extract_groups)?;

    let mut outputs = Vec::<ExtractOutput>::new();
    let mut remainder_run = Vec::<ExtractGroup>::new();

    /*
     * Die Parent-Outputs werden in der ursprünglichen Sat-Reihenfolge
     * aufgebaut. Zusammenhängende Restgruppen werden zu einem Output
     * gebündelt. Jede extrahierte Gruppe bleibt ein eigener Output.
     */
    for (index, group) in groups.iter().enumerate() {
        if selected_indices.contains(&index) {
            push_remainder_output(&mut outputs, &mut remainder_run, &req.ordinals_address)?;

            outputs.push(ExtractOutput {
                kind: ExtractOutputKind::Extracted,
                ids: group.ids.clone(),
                source_offset: group.offset,
                value: group.postage,
                address: req.ordinals_address.clone(),
            });
        } else {
            remainder_run.push(group.clone());
        }
    }

    push_remainder_output(&mut outputs, &mut remainder_run, &req.ordinals_address)?;

    let output_total = checked_output_total(&outputs)?;

    if output_total != req.total_value {
        bail!(
            "extract outputs do not preserve UTXO value: outputs {}, input {}",
            output_total,
            req.total_value
        );
    }

    let mut extracted_outputs = Vec::<ExtractedOutputRef>::new();
    let mut remainder_output_indices = Vec::<u32>::new();

    for (index, output) in outputs.iter().enumerate() {
        let output_index = u32::try_from(index).map_err(|_| anyhow!("too many extract outputs"))?;

        match output.kind {
            ExtractOutputKind::Extracted => {
                extracted_outputs.push(ExtractedOutputRef {
                    ids: output.ids.clone(),
                    source_offset: output.source_offset,
                    postage: output.value,
                    output_index,
                });
            }

            ExtractOutputKind::Remainder => {
                remainder_output_indices.push(output_index);
            }
        }
    }

    if extracted_outputs.len() != selected_indices.len() {
        bail!("not every selected group produced an extracted output");
    }

    if remainder_output_indices.is_empty() {
        bail!("extract would leave no root composition");
    }

    let remainder_groups: Vec<ExtractGroup> = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            if selected_indices.contains(&index) {
                None
            } else {
                Some(group.clone())
            }
        })
        .collect();

    let normalized_remainder = normalize_offsets(remainder_groups)?;
    let remainder_total = checked_group_total(&normalized_remainder)?;

    let extracted_total = extracted_outputs.iter().try_fold(0_u64, |total, output| {
        total
            .checked_add(output.postage)
            .ok_or_else(|| anyhow!("extracted value overflow"))
    })?;

    let reconstructed_total = remainder_total
        .checked_add(extracted_total)
        .ok_or_else(|| anyhow!("reconstructed value overflow"))?;

    if reconstructed_total != req.total_value {
        bail!(
            "reconstructed value does not match input: reconstructed {}, input {}",
            reconstructed_total,
            req.total_value
        );
    }

    let recompose = if remainder_output_indices.len() > 1 {
        Some(RecomposeIntent {
            input_output_indices: remainder_output_indices,
            groups: normalized_remainder,
            total_value: remainder_total,
        })
    } else {
        None
    };

    Ok(ExtractPlanResponse {
        ok: true,
        extractable: true,
        input_utxo: req.input_utxo,
        outputs,
        extracted_outputs,
        recompose,
    })
}

fn validate_request(req: &ExtractPlanRequest) -> Result<()> {
    if req.input_utxo.trim().is_empty() {
        bail!("missing input UTXO");
    }

    if req.ordinals_address.trim().is_empty() {
        bail!("missing ordinals address");
    }

    if req.total_value == 0 {
        bail!("input UTXO has zero value");
    }

    if req.groups.len() < 2 {
        bail!("not composed");
    }

    if req.extract_groups.is_empty() {
        bail!("missing extract groups");
    }

    Ok(())
}

fn resolve_extract_groups(
    groups: &[ExtractGroup],
    requested_groups: &[Vec<String>],
) -> Result<HashSet<usize>> {
    let mut selected_indices = HashSet::<usize>::new();

    for requested_ids in requested_groups {
        if requested_ids.is_empty() {
            bail!("extract selection contains an empty group");
        }

        let matching_indices: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter_map(|(index, group)| {
                if group.ids.as_slice() == requested_ids.as_slice() {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();

        let index = match matching_indices.as_slice() {
            [] => bail!("extract group not found: {:?}", requested_ids),
            [index] => *index,
            _ => bail!("extract group is ambiguous: {:?}", requested_ids),
        };

        if index == 0 {
            bail!("root group cannot be extracted");
        }

        if !selected_indices.insert(index) {
            bail!("extract group selected more than once: {:?}", requested_ids);
        }
    }

    Ok(selected_indices)
}

fn validate_geometry(groups: &[ExtractGroup], total_value: u64) -> Result<()> {
    let first = groups
        .first()
        .ok_or_else(|| anyhow!("UTXO contains no groups"))?;

    if first.offset != 0 {
        bail!("first offset must be 0");
    }

    let mut seen_ids = HashSet::<String>::new();

    for group in groups {
        if group.ids.is_empty() {
            bail!("extract group has no IDs");
        }

        if group.postage == 0 {
            bail!("group at offset {} has zero postage", group.offset);
        }

        for id in &group.ids {
            if id.trim().is_empty() {
                bail!("group contains an empty ID");
            }

            if !seen_ids.insert(id.clone()) {
                bail!("duplicate inscription ID: {}", id);
            }
        }
    }

    for pair in groups.windows(2) {
        let current = &pair[0];
        let next = &pair[1];

        let expected_offset = current
            .offset
            .checked_add(current.postage)
            .ok_or_else(|| anyhow!("group boundary overflow"))?;

        if next.offset != expected_offset {
            bail!(
                "invalid UTXO geometry: expected offset {}, got {}",
                expected_offset,
                next.offset
            );
        }
    }

    let last = groups
        .last()
        .ok_or_else(|| anyhow!("UTXO contains no groups"))?;

    let calculated_total = last
        .offset
        .checked_add(last.postage)
        .ok_or_else(|| anyhow!("UTXO value overflow"))?;

    if calculated_total != total_value {
        bail!(
            "group geometry does not match UTXO value: calculated {}, supplied {}",
            calculated_total,
            total_value
        );
    }

    Ok(())
}

fn push_remainder_output(
    outputs: &mut Vec<ExtractOutput>,
    remainder_run: &mut Vec<ExtractGroup>,
    address: &str,
) -> Result<()> {
    if remainder_run.is_empty() {
        return Ok(());
    }

    let groups = std::mem::take(remainder_run);

    let source_offset = groups
        .first()
        .ok_or_else(|| anyhow!("empty remainder run"))?
        .offset;

    let value = checked_group_total(&groups)?;

    outputs.push(ExtractOutput {
        kind: ExtractOutputKind::Remainder,
        ids: flatten_ids(&groups),
        source_offset,
        value,
        address: address.to_string(),
    });

    Ok(())
}

fn normalize_offsets(mut groups: Vec<ExtractGroup>) -> Result<Vec<ExtractGroup>> {
    let mut next_offset = 0_u64;

    for group in &mut groups {
        group.offset = next_offset;

        next_offset = next_offset
            .checked_add(group.postage)
            .ok_or_else(|| anyhow!("normalized offset overflow"))?;
    }

    Ok(groups)
}

fn checked_group_total(groups: &[ExtractGroup]) -> Result<u64> {
    groups.iter().try_fold(0_u64, |total, group| {
        total
            .checked_add(group.postage)
            .ok_or_else(|| anyhow!("group value overflow"))
    })
}

fn checked_output_total(outputs: &[ExtractOutput]) -> Result<u64> {
    outputs.iter().try_fold(0_u64, |total, output| {
        total
            .checked_add(output.value)
            .ok_or_else(|| anyhow!("output value overflow"))
    })
}

fn flatten_ids(groups: &[ExtractGroup]) -> Vec<String> {
    groups
        .iter()
        .flat_map(|group| group.ids.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_request(extract_groups: Vec<Vec<String>>) -> ExtractPlanRequest {
        ExtractPlanRequest {
            input_utxo: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0"
                .to_string(),
            ordinals_address: "bc1qznl7wxgtemt5eprmr6g3yj7nn7xh5gtzuvezuz".to_string(),
            total_value: 2646,
            groups: vec![
                ExtractGroup {
                    ids: vec!["A".to_string()],
                    offset: 0,
                    postage: 546,
                },
                ExtractGroup {
                    ids: vec!["B".to_string()],
                    offset: 546,
                    postage: 700,
                },
                ExtractGroup {
                    ids: vec!["C".to_string()],
                    offset: 1246,
                    postage: 600,
                },
                ExtractGroup {
                    ids: vec!["D".to_string()],
                    offset: 1846,
                    postage: 800,
                },
            ],
            extract_groups,
        }
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn separated_multi_extract_builds_recompose() {
        let plan = run_extract_plan(test_request(vec![ids(&["B"]), ids(&["D"])]))
            .expect("multi-extract must succeed");

        let kinds: Vec<ExtractOutputKind> = plan.outputs.iter().map(|output| output.kind).collect();

        assert_eq!(
            kinds,
            vec![
                ExtractOutputKind::Remainder,
                ExtractOutputKind::Extracted,
                ExtractOutputKind::Remainder,
                ExtractOutputKind::Extracted,
            ]
        );

        assert_eq!(plan.extracted_outputs.len(), 2);
        assert_eq!(plan.extracted_outputs[0].output_index, 1);
        assert_eq!(plan.extracted_outputs[1].output_index, 3);

        let recompose = plan.recompose.expect("recompose must exist");

        assert_eq!(recompose.input_output_indices, vec![0, 2]);

        assert_eq!(recompose.total_value, 1146);
        assert_eq!(recompose.groups[0].ids, ids(&["A"]));
        assert_eq!(recompose.groups[0].offset, 0);
        assert_eq!(recompose.groups[1].ids, ids(&["C"]));
        assert_eq!(recompose.groups[1].offset, 546);
    }

    #[test]
    fn adjacent_extracts_do_not_create_empty_remainder() {
        let plan = run_extract_plan(test_request(vec![ids(&["B"]), ids(&["C"])]))
            .expect("adjacent extract must succeed");

        assert_eq!(plan.outputs.len(), 4);

        assert_eq!(
            plan.outputs
                .iter()
                .map(|output| output.kind)
                .collect::<Vec<_>>(),
            vec![
                ExtractOutputKind::Remainder,
                ExtractOutputKind::Extracted,
                ExtractOutputKind::Extracted,
                ExtractOutputKind::Remainder,
            ]
        );

        let recompose = plan.recompose.expect("recompose must exist");

        assert_eq!(recompose.input_output_indices, vec![0, 3]);

        assert_eq!(recompose.total_value, 1346);
    }

    #[test]
    fn suffix_extract_needs_no_recompose() {
        let plan = run_extract_plan(test_request(vec![ids(&["B"]), ids(&["C"]), ids(&["D"])]))
            .expect("suffix extract must succeed");

        assert!(plan.recompose.is_none());
        assert_eq!(plan.outputs.len(), 4);
        assert_eq!(plan.extracted_outputs.len(), 3);

        assert_eq!(plan.outputs[0].kind, ExtractOutputKind::Remainder);
        assert_eq!(plan.outputs[0].value, 546);
    }

    #[test]
    fn root_extract_is_rejected() {
        let error =
            run_extract_plan(test_request(vec![ids(&["A"])])).expect_err("root extract must fail");

        assert!(error.to_string().contains("root group cannot be extracted"));
    }

    #[test]
    fn invalid_offset_geometry_is_rejected() {
        let mut request = test_request(vec![ids(&["C"])]);

        request.groups[1].offset = 547;
        request.groups[1].postage = 699;

        let error = run_extract_plan(request).expect_err("invalid geometry must fail");

        assert!(error
            .to_string()
            .contains("invalid UTXO geometry: expected offset 546, got 547"));
    }

    #[test]
    fn duplicate_extract_selection_is_rejected() {
        let error = run_extract_plan(test_request(vec![ids(&["B"]), ids(&["B"])]))
            .expect_err("duplicate selection must fail");

        assert!(error.to_string().contains("selected more than once"));
    }
}
