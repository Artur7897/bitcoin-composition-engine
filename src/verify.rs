use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::extract::ExtractGroup;
use crate::insert::ExistingInsertGroup;
use crate::models::{Inscription, Utxo};
use crate::spec::{read_structural_spec, Direction, StructuralSpec};
use crate::split_types::SplitGroup;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifyNodeIntent {
    pub id: String,

    #[serde(default)]
    pub children: Vec<VerifyNodeIntent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyCompositionRequest {
    pub utxo: Utxo,
    pub intent: VerifyNodeIntent,

    /*
     * Embedded specs indexed by inscription ID.
     *
     * Only nodes with children necessarily require
     * a structural spec.
     */
    #[serde(default)]
    pub specs: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedItem {
    pub id: String,
    pub level: String,
    pub parent_id: Option<String>,
    pub offset: u64,
    pub postage: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedGroup {
    /*
     * Semantic root of this group.
     *
     * The semantic root may, for example,
     * be a different ID even though the contained
     * ordinal physically precedes it.
     */
    pub root_id: String,

    /*
     * IDs in required physical sat order.
     */
    pub ids: Vec<String>,

    pub offset: u64,
    pub postage: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyCompositionResponse {
    pub ok: bool,
    pub valid: bool,

    pub input_utxo: String,
    pub root_id: String,
    pub total_value: u64,

    /*
     * Individual on-chain geometry.
     */
    pub items: Vec<VerifiedItem>,

    /*
     * Semantic top-level groups already translated
     * into BCE's flat physical representation.
     */
    pub groups: Vec<VerifiedGroup>,
}

impl VerifyCompositionResponse {
    pub fn split_groups(&self) -> Vec<SplitGroup> {
        self.groups
            .iter()
            .map(|group| SplitGroup {
                ids: group.ids.clone(),
                offset: group.offset,
                value: group.postage,
            })
            .collect()
    }

    pub fn extract_groups(&self) -> Vec<ExtractGroup> {
        self.groups
            .iter()
            .map(|group| ExtractGroup {
                ids: group.ids.clone(),
                offset: group.offset,
                postage: group.postage,
            })
            .collect()
    }

    pub fn existing_insert_groups(&self) -> Vec<ExistingInsertGroup> {
        self.groups
            .iter()
            .map(|group| ExistingInsertGroup {
                ids: group.ids.clone(),
                offset: group.offset,
                postage: group.postage,
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct FlatIntentNode {
    id: String,
    parent_id: Option<String>,
    depth: usize,
    children: Vec<String>,
}

#[derive(Debug, Clone)]
struct ChainItem {
    id: String,
    offset: u64,
    postage: u64,
}

#[derive(Debug, Clone)]
struct SubtreeInfo {
    ids: Vec<String>,
    offset: u64,
    end: u64,
    postage: u64,
}

pub fn verify_composition(req: VerifyCompositionRequest) -> Result<VerifyCompositionResponse> {
    validate_utxo(&req.utxo)?;

    let mut flat_intent = BTreeMap::<String, FlatIntentNode>::new();

    flatten_intent(&req.intent, None, 0, &mut flat_intent)?;

    let observed_chain_items = build_chain_items(&req.utxo)?;

    validate_same_ids(&flat_intent, &observed_chain_items)?;

    let chain_items: Vec<ChainItem> = observed_chain_items
        .iter()
        .filter(|item| flat_intent.contains_key(&item.id))
        .map(|item| ChainItem {
            id: item.id.clone(),
            offset: item.offset,
            postage: item.postage,
        })
        .collect();

    let chain_index: HashMap<String, usize> = chain_items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id.clone(), index))
        .collect();

    let verified_items = build_verified_items(&flat_intent, &chain_items)?;

    /*
     * Every subtree must occupy a contiguous
     * physical region. Only then can Extract
     * treat it deterministically as a range.
     */
    for node_id in flat_intent.keys() {
        build_subtree_info(node_id, &flat_intent, &chain_items, &chain_index)?;
    }

    validate_semantic_relations(&flat_intent, &chain_items, &chain_index, &req.specs)?;

    let groups = build_core_groups(
        &req.intent.id,
        &flat_intent,
        &chain_items,
        &chain_index,
        req.utxo.value,
    )?;

    Ok(VerifyCompositionResponse {
        ok: true,
        valid: true,
        input_utxo: req.utxo.outpoint,
        root_id: req.intent.id,
        total_value: req.utxo.value,
        items: verified_items,
        groups,
    })
}

fn validate_utxo(utxo: &Utxo) -> Result<()> {
    if utxo.outpoint.trim().is_empty() {
        bail!("missing UTXO outpoint");
    }

    if utxo.value == 0 {
        bail!("UTXO has zero value");
    }

    if utxo.inscriptions.is_empty() {
        bail!("UTXO contains no inscriptions");
    }

    Ok(())
}

fn flatten_intent(
    node: &VerifyNodeIntent,
    parent_id: Option<&str>,
    depth: usize,
    flat: &mut BTreeMap<String, FlatIntentNode>,
) -> Result<()> {
    if depth >= 10 {
        bail!("semantic tree exceeds maximum level J");
    }

    if node.id.trim().is_empty() {
        bail!("semantic node has empty ID");
    }

    if flat.contains_key(&node.id) {
        bail!("duplicate semantic node ID: {}", node.id);
    }

    let child_ids = node.children.iter().map(|child| child.id.clone()).collect();

    flat.insert(
        node.id.clone(),
        FlatIntentNode {
            id: node.id.clone(),
            parent_id: parent_id.map(str::to_string),
            depth,
            children: child_ids,
        },
    );

    for child in &node.children {
        flatten_intent(child, Some(&node.id), depth + 1, flat)?;
    }

    Ok(())
}

fn build_chain_items(utxo: &Utxo) -> Result<Vec<ChainItem>> {
    let mut inscriptions = utxo.inscriptions.clone();

    inscriptions.sort_by(|left, right| {
        left.satpoint
            .offset
            .cmp(&right.satpoint.offset)
            .then_with(|| left.id.cmp(&right.id))
    });

    let first = inscriptions
        .first()
        .ok_or_else(|| anyhow!("UTXO contains no inscriptions"))?;

    if first.satpoint.offset != 0 {
        bail!(
            "first inscription must start at offset 0, got {}",
            first.satpoint.offset
        );
    }

    validate_inscription_outpoints(&inscriptions, &utxo.outpoint)?;

    let mut seen_ids = HashSet::new();
    let mut items = Vec::with_capacity(inscriptions.len());

    for (index, inscription) in inscriptions.iter().enumerate() {
        if !seen_ids.insert(inscription.id.clone()) {
            bail!("duplicate on-chain inscription ID: {}", inscription.id);
        }

        let offset = inscription.satpoint.offset;

        if offset >= utxo.value {
            bail!(
                "inscription {} offset {} exceeds UTXO value {}",
                inscription.id,
                offset,
                utxo.value
            );
        }

        let end = inscriptions
            .iter()
            .skip(index + 1)
            .find(|next| next.satpoint.offset > offset)
            .map(|next| next.satpoint.offset)
            .unwrap_or(utxo.value);

        let postage = end
            .checked_sub(offset)
            .ok_or_else(|| anyhow!("invalid inscription boundary for {}", inscription.id))?;

        if postage == 0 {
            bail!("inscription {} has zero postage", inscription.id);
        }

        items.push(ChainItem {
            id: inscription.id.clone(),
            offset,
            postage,
        });
    }

    Ok(items)
}

fn validate_inscription_outpoints(
    inscriptions: &[Inscription],
    expected_outpoint: &str,
) -> Result<()> {
    for inscription in inscriptions {
        let actual_outpoint = format!(
            "{}:{}",
            inscription.satpoint.txid, inscription.satpoint.vout
        );

        if actual_outpoint != expected_outpoint {
            bail!(
                "inscription {} belongs to {}, expected {}",
                inscription.id,
                actual_outpoint,
                expected_outpoint
            );
        }
    }

    Ok(())
}

fn validate_same_ids(
    intent: &BTreeMap<String, FlatIntentNode>,
    chain_items: &[ChainItem],
) -> Result<()> {
    let intent_ids: BTreeSet<String> = intent.keys().cloned().collect();

    let chain_ids: BTreeSet<String> = chain_items.iter().map(|item| item.id.clone()).collect();

    let missing: Vec<String> = intent_ids.difference(&chain_ids).cloned().collect();

    if !missing.is_empty() {
        bail!("semantic IDs missing on-chain: {}", missing.join(", "));
    }

    let selected_offsets: HashSet<u64> = chain_items
        .iter()
        .filter(|item| intent.contains_key(&item.id))
        .map(|item| item.offset)
        .collect();

    let unexpected: Vec<String> = chain_items
        .iter()
        .filter(|item| !intent.contains_key(&item.id) && !selected_offsets.contains(&item.offset))
        .map(|item| item.id.clone())
        .collect();

    if !unexpected.is_empty() {
        bail!(
            "unexpected on-chain IDs at unselected physical offsets: {}",
            unexpected.join(", ")
        );
    }

    Ok(())
}

fn build_verified_items(
    intent: &BTreeMap<String, FlatIntentNode>,
    chain_items: &[ChainItem],
) -> Result<Vec<VerifiedItem>> {
    chain_items
        .iter()
        .map(|chain_item| {
            let node = intent
                .get(&chain_item.id)
                .ok_or_else(|| anyhow!("missing semantic node {}", chain_item.id))?;

            Ok(VerifiedItem {
                id: chain_item.id.clone(),
                level: level_for_depth(node.depth)?,
                parent_id: node.parent_id.clone(),
                offset: chain_item.offset,
                postage: chain_item.postage,
            })
        })
        .collect()
}

fn validate_semantic_relations(
    intent: &BTreeMap<String, FlatIntentNode>,
    chain_items: &[ChainItem],
    chain_index: &HashMap<String, usize>,
    specs: &BTreeMap<String, Value>,
) -> Result<()> {
    for node in intent.values() {
        if node.children.is_empty() {
            continue;
        }

        let spec_value = specs
            .get(&node.id)
            .ok_or_else(|| anyhow!("node {} has children but no structural spec", node.id))?;

        let spec = read_structural_spec(spec_value)?;

        validate_node_children(node, &spec, intent, chain_items, chain_index)?;
    }

    Ok(())
}

fn validate_node_children(
    node: &FlatIntentNode,
    spec: &StructuralSpec,
    intent: &BTreeMap<String, FlatIntentNode>,
    chain_items: &[ChainItem],
    chain_index: &HashMap<String, usize>,
) -> Result<()> {
    let parent_index = *chain_index
        .get(&node.id)
        .ok_or_else(|| anyhow!("missing parent {} on-chain", node.id))?;

    let parent = &chain_items[parent_index];

    let parent_end = parent
        .offset
        .checked_add(parent.postage)
        .ok_or_else(|| anyhow!("parent range overflow"))?;

    let mut direction_counts = HashMap::<Direction, usize>::new();

    let mut previous_end = None;

    for child_id in &node.children {
        let subtree = build_subtree_info(child_id, intent, chain_items, chain_index)?;

        if let Some(end) = previous_end {
            if subtree.offset < end {
                bail!("children of {} are not in physical order", node.id);
            }
        }

        previous_end = Some(subtree.end);

        let direction = if subtree.end <= parent.offset {
            Direction::Negative
        } else if subtree.offset >= parent_end {
            Direction::Positive
        } else {
            bail!(
                "child subtree {} crosses parent {} range",
                child_id,
                node.id
            );
        };

        let relation = spec
            .relations
            .iter()
            .find(|relation| {
                relation.parent_level == spec.root_level && relation.direction == direction
            })
            .ok_or_else(|| {
                anyhow!(
                    "spec of {} does not allow child {} on direction {}",
                    node.id,
                    child_id,
                    direction_symbol(direction)
                )
            })?;

        let count = direction_counts.entry(direction).or_insert(0);

        *count = count
            .checked_add(1)
            .ok_or_else(|| anyhow!("child count overflow for {}", node.id))?;

        if *count > relation.max_children {
            bail!(
                "node {} exceeds maxChildren {} on direction {}",
                node.id,
                relation.max_children,
                direction_symbol(direction)
            );
        }
    }

    Ok(())
}

fn build_subtree_info(
    root_id: &str,
    intent: &BTreeMap<String, FlatIntentNode>,
    chain_items: &[ChainItem],
    chain_index: &HashMap<String, usize>,
) -> Result<SubtreeInfo> {
    let mut subtree_ids = Vec::new();

    collect_subtree_ids(root_id, intent, &mut subtree_ids)?;

    let mut indices = Vec::with_capacity(subtree_ids.len());

    for id in subtree_ids {
        let index = *chain_index
            .get(&id)
            .ok_or_else(|| anyhow!("subtree ID {} missing on-chain", id))?;

        indices.push(index);
    }

    indices.sort_unstable();

    let first_index = *indices
        .first()
        .ok_or_else(|| anyhow!("empty subtree {}", root_id))?;

    let last_index = *indices
        .last()
        .ok_or_else(|| anyhow!("empty subtree {}", root_id))?;

    let expected_len = last_index
        .checked_sub(first_index)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| anyhow!("subtree index overflow"))?;

    if expected_len != indices.len() {
        bail!("subtree {} is not physically contiguous", root_id);
    }

    for pair in indices.windows(2) {
        if pair[1] != pair[0] + 1 {
            bail!("subtree {} is not physically contiguous", root_id);
        }
    }

    let first = &chain_items[first_index];
    let last = &chain_items[last_index];

    let end = last
        .offset
        .checked_add(last.postage)
        .ok_or_else(|| anyhow!("subtree end overflow"))?;

    let postage = end
        .checked_sub(first.offset)
        .ok_or_else(|| anyhow!("subtree postage overflow"))?;

    let ids = indices
        .iter()
        .map(|index| chain_items[*index].id.clone())
        .collect();

    Ok(SubtreeInfo {
        ids,
        offset: first.offset,
        end,
        postage,
    })
}

fn collect_subtree_ids(
    root_id: &str,
    intent: &BTreeMap<String, FlatIntentNode>,
    output: &mut Vec<String>,
) -> Result<()> {
    let node = intent
        .get(root_id)
        .ok_or_else(|| anyhow!("missing semantic subtree root {}", root_id))?;

    output.push(root_id.to_string());

    for child_id in &node.children {
        collect_subtree_ids(child_id, intent, output)?;
    }

    Ok(())
}

fn build_core_groups(
    root_id: &str,
    intent: &BTreeMap<String, FlatIntentNode>,
    chain_items: &[ChainItem],
    chain_index: &HashMap<String, usize>,
    total_value: u64,
) -> Result<Vec<VerifiedGroup>> {
    let root = intent
        .get(root_id)
        .ok_or_else(|| anyhow!("missing semantic root {}", root_id))?;

    let root_index = *chain_index
        .get(root_id)
        .ok_or_else(|| anyhow!("semantic root {} missing on-chain", root_id))?;

    let root_item = &chain_items[root_index];

    let mut groups = vec![VerifiedGroup {
        root_id: root_id.to_string(),
        ids: vec![root_id.to_string()],
        offset: root_item.offset,
        postage: root_item.postage,
    }];

    for child_id in &root.children {
        let subtree = build_subtree_info(child_id, intent, chain_items, chain_index)?;

        groups.push(VerifiedGroup {
            root_id: child_id.clone(),
            ids: subtree.ids,
            offset: subtree.offset,
            postage: subtree.postage,
        });
    }

    groups.sort_by_key(|group| group.offset);

    let first = groups
        .first()
        .ok_or_else(|| anyhow!("verified composition has no groups"))?;

    if first.offset != 0 {
        bail!("first verified group must start at offset 0");
    }

    let mut next_offset = 0_u64;

    for group in &groups {
        if group.offset != next_offset {
            bail!(
                "verified group geometry gap: expected {}, got {}",
                next_offset,
                group.offset
            );
        }

        next_offset = next_offset
            .checked_add(group.postage)
            .ok_or_else(|| anyhow!("verified group offset overflow"))?;
    }

    if next_offset != total_value {
        bail!(
            "verified groups total {} does not match UTXO value {}",
            next_offset,
            total_value
        );
    }

    Ok(groups)
}

fn level_for_depth(depth: usize) -> Result<String> {
    if depth >= 10 {
        bail!("semantic depth exceeds level J");
    }

    let depth = u8::try_from(depth).map_err(|_| anyhow!("semantic depth overflow"))?;

    let byte = b'A'
        .checked_add(depth)
        .ok_or_else(|| anyhow!("semantic level overflow"))?;

    Ok(char::from(byte).to_string())
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
    use crate::models::SatPoint;
    use serde_json::json;

    const TXID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn inscription(id: &str, offset: u64) -> Inscription {
        Inscription {
            id: id.to_string(),
            satpoint: SatPoint {
                txid: TXID.to_string(),
                vout: 0,
                offset,
            },
        }
    }

    fn one_child_spec(direction: &str) -> Value {
        json!({
            "structure": {
                "version": 1,
                "rootLevel": "A",
                "relations": [
                    {
                        "parentLevel": "A",
                        "childLevel": "B",
                        "direction": direction,
                        "maxChildren": 1
                    }
                ]
            }
        })
    }

    fn multi_child_spec() -> Value {
        json!({
            "structure": {
                "version": 1,
                "rootLevel": "A",
                "relations": [
                    {
                        "parentLevel": "A",
                        "childLevel": "B",
                        "direction": "+",
                        "maxChildren": 48
                    }
                ]
            }
        })
    }

    #[test]
    fn semantic_root_may_have_non_zero_offset() {
        let mut specs = BTreeMap::new();
        specs.insert("NODE".to_string(), one_child_spec("-"));

        let result = verify_composition(VerifyCompositionRequest {
            utxo: Utxo {
                outpoint: format!("{TXID}:0"),
                value: 1146,
                address: "bc1ptest".to_string(),
                inscriptions: vec![inscription("LEAF", 0), inscription("NODE", 600)],
            },
            intent: VerifyNodeIntent {
                id: "NODE".to_string(),
                children: vec![VerifyNodeIntent {
                    id: "LEAF".to_string(),
                    children: vec![],
                }],
            },
            specs,
        })
        .unwrap();

        assert_eq!(result.root_id, "NODE");
        assert_eq!(result.items[1].offset, 600);
        assert_eq!(result.groups.len(), 2);
        assert_eq!(result.groups[0].ids, vec!["LEAF"]);
        assert_eq!(result.groups[1].ids, vec!["NODE"]);
    }

    #[test]
    fn nested_subtrees_become_contiguous_groups() {
        let mut specs = BTreeMap::new();

        specs.insert("ROOT".to_string(), multi_child_spec());

        specs.insert("NODE-1".to_string(), one_child_spec("-"));

        specs.insert("NODE-2".to_string(), one_child_spec("-"));

        let result = verify_composition(VerifyCompositionRequest {
            utxo: Utxo {
                outpoint: format!("{TXID}:0"),
                value: 2838,
                address: "bc1ptest".to_string(),
                inscriptions: vec![
                    inscription("ROOT", 0),
                    inscription("LEAF-1", 546),
                    inscription("NODE-1", 1146),
                    inscription("LEAF-2", 1692),
                    inscription("NODE-2", 2292),
                ],
            },
            intent: VerifyNodeIntent {
                id: "ROOT".to_string(),
                children: vec![
                    VerifyNodeIntent {
                        id: "NODE-1".to_string(),
                        children: vec![VerifyNodeIntent {
                            id: "LEAF-1".to_string(),
                            children: vec![],
                        }],
                    },
                    VerifyNodeIntent {
                        id: "NODE-2".to_string(),
                        children: vec![VerifyNodeIntent {
                            id: "LEAF-2".to_string(),
                            children: vec![],
                        }],
                    },
                ],
            },
            specs,
        })
        .unwrap();

        assert_eq!(result.groups.len(), 3);

        assert_eq!(result.groups[1].ids, vec!["LEAF-1", "NODE-1"]);

        assert_eq!(result.groups[1].postage, 1146);

        assert_eq!(result.groups[2].ids, vec!["LEAF-2", "NODE-2"]);
    }

    #[test]
    fn rejects_wrong_direction() {
        let mut specs = BTreeMap::new();

        specs.insert("NODE".to_string(), one_child_spec("+"));

        let error = verify_composition(VerifyCompositionRequest {
            utxo: Utxo {
                outpoint: format!("{TXID}:0"),
                value: 1146,
                address: "bc1ptest".to_string(),
                inscriptions: vec![inscription("LEAF", 0), inscription("NODE", 600)],
            },
            intent: VerifyNodeIntent {
                id: "NODE".to_string(),
                children: vec![VerifyNodeIntent {
                    id: "LEAF".to_string(),
                    children: vec![],
                }],
            },
            specs,
        })
        .expect_err("wrong direction must fail");

        assert!(error.to_string().contains("does not allow"));
    }

    #[test]
    fn rejects_missing_on_chain_id() {
        let mut specs = BTreeMap::new();

        specs.insert("NODE".to_string(), one_child_spec("-"));

        let error = verify_composition(VerifyCompositionRequest {
            utxo: Utxo {
                outpoint: format!("{TXID}:0"),
                value: 1146,
                address: "bc1ptest".to_string(),
                inscriptions: vec![inscription("LEAF", 0), inscription("NODE", 600)],
            },
            intent: VerifyNodeIntent {
                id: "NODE".to_string(),
                children: vec![VerifyNodeIntent {
                    id: "MISSING".to_string(),
                    children: vec![],
                }],
            },
            specs,
        })
        .expect_err("missing ID must fail");

        assert!(error.to_string().contains("missing on-chain"));
    }

    #[test]
    fn rejects_interleaved_subtrees() {
        let mut specs = BTreeMap::new();

        specs.insert("ROOT".to_string(), multi_child_spec());

        specs.insert("NODE-1".to_string(), one_child_spec("-"));

        specs.insert("NODE-2".to_string(), one_child_spec("-"));

        let error = verify_composition(VerifyCompositionRequest {
            utxo: Utxo {
                outpoint: format!("{TXID}:0"),
                value: 2838,
                address: "bc1ptest".to_string(),
                inscriptions: vec![
                    inscription("ROOT", 0),
                    inscription("LEAF-1", 546),
                    inscription("LEAF-2", 1146),
                    inscription("NODE-1", 1746),
                    inscription("NODE-2", 2292),
                ],
            },
            intent: VerifyNodeIntent {
                id: "ROOT".to_string(),
                children: vec![
                    VerifyNodeIntent {
                        id: "NODE-1".to_string(),
                        children: vec![VerifyNodeIntent {
                            id: "LEAF-1".to_string(),
                            children: vec![],
                        }],
                    },
                    VerifyNodeIntent {
                        id: "NODE-2".to_string(),
                        children: vec![VerifyNodeIntent {
                            id: "LEAF-2".to_string(),
                            children: vec![],
                        }],
                    },
                ],
            },
            specs,
        })
        .expect_err("interleaved subtrees must fail");

        assert!(error.to_string().contains("not physically contiguous"));
    }

    #[test]
    fn translates_verified_groups_for_core() {
        let mut specs = BTreeMap::new();

        specs.insert("NODE".to_string(), one_child_spec("-"));

        let result = verify_composition(VerifyCompositionRequest {
            utxo: Utxo {
                outpoint: format!("{TXID}:0"),
                value: 1146,
                address: "bc1ptest".to_string(),
                inscriptions: vec![inscription("LEAF", 0), inscription("NODE", 600)],
            },
            intent: VerifyNodeIntent {
                id: "NODE".to_string(),
                children: vec![VerifyNodeIntent {
                    id: "LEAF".to_string(),
                    children: vec![],
                }],
            },
            specs,
        })
        .unwrap();

        let split = result.split_groups();
        let extract = result.extract_groups();
        let insert = result.existing_insert_groups();

        assert_eq!(split[0].value, 600);
        assert_eq!(extract[0].postage, 600);
        assert_eq!(insert[0].postage, 600);
    }

    #[test]
    fn shared_satpoint_uses_one_physical_span() {
        let utxo = Utxo {
            outpoint: format!("{TXID}:0"),
            value: 1546,
            address: "bc1ptest".to_string(),
            inscriptions: vec![
                inscription("A", 0),
                inscription("B", 0),
                inscription("C", 1000),
            ],
        };

        let items = build_chain_items(&utxo).unwrap();

        assert_eq!(items.len(), 3);

        assert_eq!(items[0].offset, 0);
        assert_eq!(items[0].postage, 1000);

        assert_eq!(items[1].offset, 0);
        assert_eq!(items[1].postage, 1000);

        assert_eq!(items[2].offset, 1000);
        assert_eq!(items[2].postage, 546);
    }

    #[test]
    fn co_satpoint_id_does_not_become_required_semantic_node() {
        let mut intent = BTreeMap::new();

        intent.insert(
            "A".to_string(),
            FlatIntentNode {
                id: "A".to_string(),
                parent_id: None,
                depth: 0,
                children: vec![],
            },
        );

        let shared = vec![
            ChainItem {
                id: "A".to_string(),
                offset: 0,
                postage: 1000,
            },
            ChainItem {
                id: "B".to_string(),
                offset: 0,
                postage: 1000,
            },
        ];

        validate_same_ids(&intent, &shared).expect("co-satpoint inscription must be allowed");

        let separate = vec![
            ChainItem {
                id: "A".to_string(),
                offset: 0,
                postage: 1000,
            },
            ChainItem {
                id: "B".to_string(),
                offset: 1000,
                postage: 546,
            },
        ];

        let error = validate_same_ids(&intent, &separate)
            .expect_err("unselected physical position must fail");

        assert!(error.to_string().contains("unselected physical offsets"));
    }
}
