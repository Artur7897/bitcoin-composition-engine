use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::compose_plan::run_compose_plan;
use crate::compose_types::{ComposePlanItem, ComposePlanRequest, ComposePlanResponse};
use crate::models::Utxo;
use crate::spec::{read_structural_spec, Direction, StructuralSpec};
use crate::verify::{verify_composition, VerifiedItem, VerifyCompositionRequest, VerifyNodeIntent};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifyComposeNodeIntent {
    pub id: String,

    /*
     * Required only when the parent spec
     * allows both + and - directions.
     */
    #[serde(default)]
    pub direction: Option<Direction>,

    #[serde(default)]
    pub children: Vec<VerifyComposeNodeIntent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyComposeRequest {
    pub intent: VerifyComposeNodeIntent,
    pub sources: Vec<Utxo>,

    #[serde(default)]
    pub specs: BTreeMap<String, Value>,

    pub fee_rate: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedComposeInput {
    pub outpoint: String,

    /*
     * IDs within this input in their
     * already existing sat order.
     */
    pub ids: Vec<String>,

    /*
     * Future offset of this complete input
     * within the new UTXO.
     */
    pub offset: u64,

    pub postage: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedComposeItem {
    pub id: String,
    pub source_utxo: String,
    pub offset: u64,
    pub postage: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyComposeResponse {
    pub ok: bool,
    pub valid: bool,

    pub semantic_root_id: String,
    pub physical_root_id: String,

    pub total_value: u64,

    pub ordered_inputs: Vec<VerifiedComposeInput>,
    pub items: Vec<VerifiedComposeItem>,

    /*
     * Plan produced independently by BCE.
     * Its offsets are compared against the verified semantic intent.
     */
    pub core_plan: ComposePlanResponse,
}

#[derive(Debug, Clone)]
struct FlatComposeNode {
    parent_id: Option<String>,
}

#[derive(Debug, Clone)]
struct SourceUnit {
    outpoint: String,
    value: u64,
    items: Vec<VerifiedItem>,
    target_start: usize,
}

type VerifiedComposeBuildResult = (
    Vec<VerifiedComposeInput>,
    Vec<VerifiedComposeItem>,
    Vec<u64>,
    u64,
);

pub fn verify_compose(req: VerifyComposeRequest) -> Result<VerifyComposeResponse> {
    if req.sources.len() < 2 {
        bail!("compose requires at least two source UTXOs");
    }

    if req.intent.direction.is_some() {
        bail!("semantic compose root cannot have a direction");
    }

    let mut flat = BTreeMap::<String, FlatComposeNode>::new();

    flatten_compose_intent(&req.intent, None, 0, &mut flat)?;

    validate_source_ids(&req.sources, &flat)?;

    /*
     * The spec language produces the expected
     * future physical ID order.
     */
    let mut expected_ids = Vec::new();

    serialize_expected_order(&req.intent, &req.specs, &mut expected_ids)?;

    validate_expected_ids(&expected_ids, &flat)?;

    /*
     * Each source UTXO is independently verified as
     * an already existing state.
     */
    let mut source_units = Vec::new();

    for source in req.sources {
        let source_ids: BTreeSet<String> = source
            .inscriptions
            .iter()
            .map(|inscription| inscription.id.clone())
            .collect();

        let source_root_id = find_source_root(&source_ids, &flat)?;

        let source_node = find_compose_node(&req.intent, &source_root_id)
            .ok_or_else(|| anyhow!("cannot find semantic source root {}", source_root_id))?;

        let source_intent = prune_intent(source_node, &source_ids)
            .ok_or_else(|| anyhow!("cannot build source intent for {}", source.outpoint))?;

        let pruned_ids = collect_verify_intent_ids(&source_intent);

        if pruned_ids != source_ids {
            bail!(
                "source {} does not contain a complete semantic subtree",
                source.outpoint
            );
        }

        let verified = verify_composition(VerifyCompositionRequest {
            utxo: source,
            intent: source_intent,
            specs: req.specs.clone(),
        })?;

        let target_start = source_target_start(&verified.items, &expected_ids)?;

        source_units.push(SourceUnit {
            outpoint: verified.input_utxo,
            value: verified.total_value,
            items: verified.items,
            target_start,
        });
    }

    source_units.sort_by_key(|unit| unit.target_start);

    validate_source_order(&source_units, &expected_ids)?;

    let physical_root_id = source_units
        .first()
        .and_then(|unit| unit.items.first())
        .map(|item| item.id.clone())
        .ok_or_else(|| anyhow!("compose has no physical root input"))?;

    let root_postage = source_units
        .first()
        .map(|unit| unit.value)
        .ok_or_else(|| anyhow!("compose has no root postage"))?;

    let compose_items = source_units
        .iter()
        .skip(1)
        .map(|unit| {
            let id = unit
                .items
                .first()
                .map(|item| item.id.clone())
                .ok_or_else(|| anyhow!("compose source {} has no items", unit.outpoint))?;

            Ok(ComposePlanItem {
                id,
                postage: unit.value,
                utxo: Some(unit.outpoint.clone()),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    /*
     * BCE builds the physical composition plan independently.
     */
    let core_plan = run_compose_plan(ComposePlanRequest {
        root_id: physical_root_id.clone(),
        root_postage,
        items: compose_items,
        fee_rate: req.fee_rate,
    })?;

    let (ordered_inputs, final_items, expected_input_offsets, total_value) =
        build_verified_result(&source_units)?;

    /*
     * The independently built BCE plan is compared with
     * the verified semantic result only after both are complete.
     */
    if core_plan.planned_offsets != expected_input_offsets {
        bail!(
            "BCE compose offsets do not match verified intent: BCE {:?}, verify {:?}",
            core_plan.planned_offsets,
            expected_input_offsets
        );
    }

    let actual_ids: Vec<String> = final_items.iter().map(|item| item.id.clone()).collect();

    if actual_ids != expected_ids {
        bail!("BCE compose item order does not match semantic intent");
    }

    Ok(VerifyComposeResponse {
        ok: true,
        valid: true,
        semantic_root_id: req.intent.id,
        physical_root_id,
        total_value,
        ordered_inputs,
        items: final_items,
        core_plan,
    })
}

fn flatten_compose_intent(
    node: &VerifyComposeNodeIntent,
    parent_id: Option<&str>,
    depth: usize,
    flat: &mut BTreeMap<String, FlatComposeNode>,
) -> Result<()> {
    if depth >= 10 {
        bail!("compose intent exceeds maximum level J");
    }

    if node.id.trim().is_empty() {
        bail!("compose intent contains empty ID");
    }

    if flat.contains_key(&node.id) {
        bail!("duplicate compose intent ID: {}", node.id);
    }

    flat.insert(
        node.id.clone(),
        FlatComposeNode {
            parent_id: parent_id.map(str::to_string),
        },
    );

    for child in &node.children {
        flatten_compose_intent(child, Some(&node.id), depth + 1, flat)?;
    }

    Ok(())
}

fn validate_source_ids(sources: &[Utxo], flat: &BTreeMap<String, FlatComposeNode>) -> Result<()> {
    let mut seen_outpoints = HashSet::new();
    let mut seen_ids = HashSet::new();

    for source in sources {
        if source.outpoint.trim().is_empty() {
            bail!("compose source has empty outpoint");
        }

        if !seen_outpoints.insert(source.outpoint.clone()) {
            bail!("duplicate compose source UTXO: {}", source.outpoint);
        }

        if source.value == 0 {
            bail!("compose source {} has zero value", source.outpoint);
        }

        for inscription in &source.inscriptions {
            if !seen_ids.insert(inscription.id.clone()) {
                bail!(
                    "inscription {} appears in multiple compose sources",
                    inscription.id
                );
            }
        }
    }

    let source_ids: BTreeSet<String> = seen_ids.into_iter().collect();

    let intent_ids: BTreeSet<String> = flat.keys().cloned().collect();

    let missing: Vec<String> = intent_ids.difference(&source_ids).cloned().collect();

    if !missing.is_empty() {
        bail!(
            "compose intent IDs missing from sources: {}",
            missing.join(", ")
        );
    }

    let unexpected: Vec<String> = source_ids.difference(&intent_ids).cloned().collect();

    if !unexpected.is_empty() {
        bail!("unexpected source IDs: {}", unexpected.join(", "));
    }

    Ok(())
}

fn serialize_expected_order(
    node: &VerifyComposeNodeIntent,
    specs: &BTreeMap<String, Value>,
    output: &mut Vec<String>,
) -> Result<()> {
    if node.children.is_empty() {
        output.push(node.id.clone());
        return Ok(());
    }

    let spec_value = specs.get(&node.id).ok_or_else(|| {
        anyhow!(
            "compose node {} has children but no structural spec",
            node.id
        )
    })?;

    let spec = read_structural_spec(spec_value)?;

    let mut negative = Vec::new();
    let mut positive = Vec::new();

    let mut counts = HashMap::<Direction, usize>::new();

    for child in &node.children {
        let direction = resolve_target_direction(child, &spec, &node.id)?;

        let relation = root_relation(&spec, direction)?;

        let count = counts.entry(direction).or_insert(0);

        *count = count
            .checked_add(1)
            .ok_or_else(|| anyhow!("compose child count overflow"))?;

        if *count > relation.max_children {
            bail!(
                "compose node {} exceeds maxChildren {} on direction {}",
                node.id,
                relation.max_children,
                direction_symbol(direction)
            );
        }

        match direction {
            Direction::Negative => {
                negative.push(child);
            }

            Direction::Positive => {
                positive.push(child);
            }
        }
    }

    for child in negative {
        serialize_expected_order(child, specs, output)?;
    }

    output.push(node.id.clone());

    for child in positive {
        serialize_expected_order(child, specs, output)?;
    }

    Ok(())
}

fn resolve_target_direction(
    child: &VerifyComposeNodeIntent,
    spec: &StructuralSpec,
    parent_id: &str,
) -> Result<Direction> {
    let allowed: Vec<Direction> = spec
        .relations
        .iter()
        .filter(|relation| relation.parent_level == spec.root_level)
        .map(|relation| relation.direction)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if let Some(direction) = child.direction {
        if !allowed.contains(&direction) {
            bail!(
                "spec of {} does not allow child {} on direction {}",
                parent_id,
                child.id,
                direction_symbol(direction)
            );
        }

        return Ok(direction);
    }

    match allowed.as_slice() {
        [direction] => Ok(*direction),

        [] => bail!("spec of {} has no child relation", parent_id),

        _ => bail!(
            "child {} must declare direction because parent {} allows both spaces",
            child.id,
            parent_id
        ),
    }
}

fn root_relation(
    spec: &StructuralSpec,
    direction: Direction,
) -> Result<&crate::spec::StructuralRelation> {
    spec.relations
        .iter()
        .find(|relation| {
            relation.parent_level == spec.root_level && relation.direction == direction
        })
        .ok_or_else(|| {
            anyhow!(
                "missing root relation on direction {}",
                direction_symbol(direction)
            )
        })
}

fn validate_expected_ids(
    expected_ids: &[String],
    flat: &BTreeMap<String, FlatComposeNode>,
) -> Result<()> {
    let expected: BTreeSet<String> = expected_ids.iter().cloned().collect();

    let intent: BTreeSet<String> = flat.keys().cloned().collect();

    if expected.len() != expected_ids.len() {
        bail!("semantic serialization produced duplicate IDs");
    }

    if expected != intent {
        bail!("semantic serialization does not contain every intent ID");
    }

    Ok(())
}

fn find_source_root(
    source_ids: &BTreeSet<String>,
    flat: &BTreeMap<String, FlatComposeNode>,
) -> Result<String> {
    let roots: Vec<String> = source_ids
        .iter()
        .filter(|id| {
            flat.get(*id)
                .and_then(|node| node.parent_id.as_ref())
                .map(|parent_id| !source_ids.contains(parent_id))
                .unwrap_or(true)
        })
        .cloned()
        .collect();

    match roots.as_slice() {
        [root] => Ok(root.clone()),

        [] => bail!("compose source contains no semantic root"),

        _ => bail!(
            "compose source contains multiple disconnected semantic roots: {}",
            roots.join(", ")
        ),
    }
}

fn find_compose_node<'a>(
    node: &'a VerifyComposeNodeIntent,
    searched_id: &str,
) -> Option<&'a VerifyComposeNodeIntent> {
    if node.id == searched_id {
        return Some(node);
    }

    for child in &node.children {
        if let Some(found) = find_compose_node(child, searched_id) {
            return Some(found);
        }
    }

    None
}

fn prune_intent(
    node: &VerifyComposeNodeIntent,
    included: &BTreeSet<String>,
) -> Option<VerifyNodeIntent> {
    if !included.contains(&node.id) {
        return None;
    }

    let children = node
        .children
        .iter()
        .filter_map(|child| prune_intent(child, included))
        .collect();

    Some(VerifyNodeIntent {
        id: node.id.clone(),
        children,
    })
}

fn collect_verify_intent_ids(node: &VerifyNodeIntent) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    collect_verify_ids(node, &mut ids);
    ids
}

fn collect_verify_ids(node: &VerifyNodeIntent, output: &mut BTreeSet<String>) {
    output.insert(node.id.clone());

    for child in &node.children {
        collect_verify_ids(child, output);
    }
}

fn source_target_start(items: &[VerifiedItem], expected_ids: &[String]) -> Result<usize> {
    let positions: HashMap<&str, usize> = expected_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();

    let first = items
        .first()
        .ok_or_else(|| anyhow!("verified source contains no items"))?;

    positions
        .get(first.id.as_str())
        .copied()
        .ok_or_else(|| anyhow!("source item {} missing from target order", first.id))
}

fn validate_source_order(units: &[SourceUnit], expected_ids: &[String]) -> Result<()> {
    let positions: HashMap<&str, usize> = expected_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();

    let mut next_position = 0_usize;

    for unit in units {
        if unit.target_start != next_position {
            bail!(
                "compose source {} does not form the next contiguous target range",
                unit.outpoint
            );
        }

        for item in &unit.items {
            let position = positions
                .get(item.id.as_str())
                .copied()
                .ok_or_else(|| anyhow!("source item {} missing from target", item.id))?;

            if position != next_position {
                bail!(
                    "compose source {} would require splitting or reordering",
                    unit.outpoint
                );
            }

            next_position = next_position
                .checked_add(1)
                .ok_or_else(|| anyhow!("compose target position overflow"))?;
        }
    }

    if next_position != expected_ids.len() {
        bail!("compose sources do not cover complete target order");
    }

    Ok(())
}

fn build_verified_result(units: &[SourceUnit]) -> Result<VerifiedComposeBuildResult> {
    let mut ordered_inputs = Vec::new();
    let mut final_items = Vec::new();
    let mut item_input_offsets = Vec::new();

    let mut next_offset = 0_u64;

    for (input_index, unit) in units.iter().enumerate() {
        if input_index > 0 {
            item_input_offsets.push(next_offset);
        }

        let ids = unit.items.iter().map(|item| item.id.clone()).collect();

        ordered_inputs.push(VerifiedComposeInput {
            outpoint: unit.outpoint.clone(),
            ids,
            offset: next_offset,
            postage: unit.value,
        });

        for item in &unit.items {
            let offset = next_offset
                .checked_add(item.offset)
                .ok_or_else(|| anyhow!("final compose item offset overflow"))?;

            final_items.push(VerifiedComposeItem {
                id: item.id.clone(),
                source_utxo: unit.outpoint.clone(),
                offset,
                postage: item.postage,
            });
        }

        next_offset = next_offset
            .checked_add(unit.value)
            .ok_or_else(|| anyhow!("final compose value overflow"))?;
    }

    final_items.sort_by_key(|item| item.offset);

    Ok((ordered_inputs, final_items, item_input_offsets, next_offset))
}

fn direction_symbol(direction: Direction) -> &'static str {
    match direction {
        Direction::Positive => "+",
        Direction::Negative => "-",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Inscription, SatPoint};
    use serde_json::json;

    fn source(txid: &str, value: u64, inscriptions: Vec<(&str, u64)>) -> Utxo {
        Utxo {
            outpoint: format!("{txid}:0"),
            value,
            address: "bc1ptest".to_string(),
            inscriptions: inscriptions
                .into_iter()
                .map(|(id, offset)| Inscription {
                    id: id.to_string(),
                    satpoint: SatPoint {
                        txid: txid.to_string(),
                        vout: 0,
                        offset,
                    },
                })
                .collect(),
        }
    }

    fn one_side_spec(direction: &str, max_children: usize) -> Value {
        json!({
            "structure": {
                "version": 1,
                "rootLevel": "A",
                "relations": [
                    {
                        "parentLevel": "A",
                        "childLevel": "B",
                        "direction": direction,
                        "maxChildren": max_children
                    }
                ]
            }
        })
    }

    #[test]
    fn negative_direction_places_child_before_semantic_root() {
        let child_txid = "b".repeat(64);
        let node_txid = "c".repeat(64);

        let mut specs = BTreeMap::new();

        specs.insert("NODE".to_string(), one_side_spec("-", 1));

        let result = verify_compose(VerifyComposeRequest {
            intent: VerifyComposeNodeIntent {
                id: "NODE".to_string(),
                direction: None,
                children: vec![VerifyComposeNodeIntent {
                    id: "LEAF".to_string(),
                    direction: None,
                    children: vec![],
                }],
            },
            sources: vec![
                source(&node_txid, 546, vec![("NODE", 0)]),
                source(&child_txid, 600, vec![("LEAF", 0)]),
            ],
            specs,
            fee_rate: Some(1),
        })
        .unwrap();

        assert_eq!(result.physical_root_id, "LEAF");

        assert_eq!(result.core_plan.planned_offsets, vec![600]);

        assert_eq!(result.items[0].id, "LEAF");
        assert_eq!(result.items[0].offset, 0);
        assert_eq!(result.items[1].id, "NODE");
        assert_eq!(result.items[1].offset, 600);
    }

    #[test]
    fn keeps_existing_subtree_together() {
        let root_txid = "a".repeat(64);
        let node_txid = "b".repeat(64);

        let mut specs = BTreeMap::new();

        specs.insert("ROOT".to_string(), one_side_spec("+", 48));

        specs.insert("NODE".to_string(), one_side_spec("-", 1));

        let result = verify_compose(VerifyComposeRequest {
            intent: VerifyComposeNodeIntent {
                id: "ROOT".to_string(),
                direction: None,
                children: vec![VerifyComposeNodeIntent {
                    id: "NODE".to_string(),
                    direction: None,
                    children: vec![VerifyComposeNodeIntent {
                        id: "LEAF".to_string(),
                        direction: None,
                        children: vec![],
                    }],
                }],
            },
            sources: vec![
                source(&node_txid, 1146, vec![("LEAF", 0), ("NODE", 600)]),
                source(&root_txid, 546, vec![("ROOT", 0)]),
            ],
            specs,
            fee_rate: Some(1),
        })
        .unwrap();

        assert_eq!(result.core_plan.planned_offsets, vec![546]);

        let ids: Vec<String> = result.items.iter().map(|item| item.id.clone()).collect();

        assert_eq!(ids, vec!["ROOT", "LEAF", "NODE"]);

        assert_eq!(result.items[1].offset, 546);
        assert_eq!(result.items[2].offset, 1146);
    }

    #[test]
    fn two_sided_spec_requires_explicit_direction() {
        let root_txid = "a".repeat(64);
        let child_txid = "b".repeat(64);

        let mut specs = BTreeMap::new();

        specs.insert(
            "ROOT".to_string(),
            json!({
                "structure": {
                    "version": 1,
                    "rootLevel": "A",
                    "relations": [
                        {
                            "parentLevel": "A",
                            "childLevel": "B",
                            "direction": "-",
                            "maxChildren": 1
                        },
                        {
                            "parentLevel": "A",
                            "childLevel": "B",
                            "direction": "+",
                            "maxChildren": 1
                        }
                    ]
                }
            }),
        );

        let error = verify_compose(VerifyComposeRequest {
            intent: VerifyComposeNodeIntent {
                id: "ROOT".to_string(),
                direction: None,
                children: vec![VerifyComposeNodeIntent {
                    id: "CHILD".to_string(),
                    direction: None,
                    children: vec![],
                }],
            },
            sources: vec![
                source(&root_txid, 546, vec![("ROOT", 0)]),
                source(&child_txid, 600, vec![("CHILD", 0)]),
            ],
            specs,
            fee_rate: Some(1),
        })
        .err()
        .expect("ambiguous direction must fail");

        assert!(error.to_string().contains("must declare direction"));
    }
}
