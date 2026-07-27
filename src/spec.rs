use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum Direction {
    #[serde(rename = "+")]
    Positive,

    #[serde(rename = "-")]
    Negative,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddedStructuralSpec {
    version: u32,
    root_level: String,
    relations: Vec<EmbeddedRelation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmbeddedRelation {
    parent_level: String,
    child_level: String,
    direction: Direction,
    max_children: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StructuralRelation {
    pub parent_level: String,
    pub child_level: String,
    pub direction: Direction,
    pub max_children: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StructuralSpec {
    pub version: u32,
    pub root_level: String,
    pub relations: Vec<StructuralRelation>,
}

impl StructuralSpec {
    pub fn relation(
        &self,
        parent_level: &str,
        child_level: &str,
        direction: Direction,
    ) -> Option<&StructuralRelation> {
        self.relations.iter().find(|relation| {
            relation.parent_level == parent_level
                && relation.child_level == child_level
                && relation.direction == direction
        })
    }
}

/*
 * Liest die semantische Struktursprache.
 *
 * Priorität:
 *
 * 1. Neue kanonische "structure"-Sprache
 * 2. Bisherige Suitcase-/Album-Sprache
 * 3. Bisherige Case-Sprache
 * 4. Bisherige Grid-Sprache
 *
 * Visuelle Felder werden vollständig ignoriert.
 */
pub fn read_structural_spec(value: &Value) -> Result<StructuralSpec> {
    if let Some(structure) = value.get("structure") {
        return read_canonical_structure(structure);
    }

    if let Some(spec) = read_legacy_collector_structure(value)? {
        return Ok(spec);
    }

    if let Some(spec) = read_legacy_case_structure(value)? {
        return Ok(spec);
    }

    if let Some(spec) = read_legacy_grid_structure(value)? {
        return Ok(spec);
    }

    bail!("spec contains no supported structural language")
}

fn read_canonical_structure(value: &Value) -> Result<StructuralSpec> {
    let embedded: EmbeddedStructuralSpec = serde_json::from_value(value.clone())
        .map_err(|error| anyhow!("invalid structural spec: {}", error))?;

    let relations = embedded
        .relations
        .into_iter()
        .map(|relation| StructuralRelation {
            parent_level: relation.parent_level,
            child_level: relation.child_level,
            direction: relation.direction,
            max_children: relation.max_children,
        })
        .collect();

    validate_structural_spec(StructuralSpec {
        version: embedded.version,
        root_level: embedded.root_level,
        relations,
    })
}

/*
 * Adapter für die bisherige Suitcase-/Album-Sprache:
 *
 * compose.root
 * compose.children
 * hierarchy
 */
fn read_legacy_collector_structure(value: &Value) -> Result<Option<StructuralSpec>> {
    if value.pointer("/compose/root").is_none() || value.pointer("/compose/children").is_none() {
        return Ok(None);
    }

    let embedded: LegacyCollectorSpec = serde_json::from_value(value.clone())
        .map_err(|error| anyhow!("invalid legacy collector spec: {}", error))?;

    if embedded.compose.children.range_type != "offset-range" {
        bail!(
            "unsupported children type: {}",
            embedded.compose.children.range_type
        );
    }

    if let Some(hierarchy) = embedded.hierarchy.as_ref() {
        if hierarchy.root_level != embedded.compose.root.level {
            bail!(
                "compose root level {} does not match hierarchy root level {}",
                embedded.compose.root.level,
                hierarchy.root_level
            );
        }

        if hierarchy.child_level != embedded.compose.children.level {
            bail!(
                "compose child level {} does not match hierarchy child level {}",
                embedded.compose.children.level,
                hierarchy.child_level
            );
        }

        if hierarchy.direction != embedded.compose.children.side {
            bail!("compose side does not match hierarchy direction");
        }
    }

    let compose_limit = embedded.compose.children.max_items;

    let hierarchy_limit = embedded
        .hierarchy
        .as_ref()
        .and_then(|hierarchy| hierarchy.max_groups);

    let max_children = match (compose_limit, hierarchy_limit) {
        (Some(compose), Some(hierarchy)) if compose != hierarchy => {
            bail!(
                "compose maxItems {} does not match hierarchy maxGroups {}",
                compose,
                hierarchy
            );
        }

        (Some(value), _) | (_, Some(value)) => value,

        (None, None) => {
            bail!("legacy structural spec has no group limit");
        }
    };

    let spec = single_relation_spec(
        embedded.compose.root.level,
        embedded.compose.children.level,
        embedded.compose.children.side,
        max_children,
    );

    validate_structural_spec(spec).map(Some)
}

/*
 * Adapter für die bisherige Case-Sprache:
 *
 * compose.role = append-layout
 * compose.displayedContent.direction = -
 * slotCount = 1
 *
 * Die Case-Inskription ist der semantische Root.
 * Das dargestellte Ordinal liegt auf ihrer negativen Seite.
 */
fn read_legacy_case_structure(value: &Value) -> Result<Option<StructuralSpec>> {
    let role = value.pointer("/compose/role").and_then(Value::as_str);

    if role != Some("append-layout") {
        return Ok(None);
    }

    let direction_value = value
        .pointer("/compose/displayedContent/direction")
        .ok_or_else(|| anyhow!("legacy case spec has no displayed content direction"))?;

    let direction = read_direction(direction_value, "case direction")?;

    let max_children = read_slot_count(value)?;

    let spec = single_relation_spec("A".to_string(), "B".to_string(), direction, max_children);

    validate_structural_spec(spec).map(Some)
}

/*
 * Adapter für die bisherige Display-Grid-Sprache.
 *
 * Ein Grid hat keine alte compose-Struktur.
 * Der Grid-Root liegt vor seinen Slots.
 */
fn read_legacy_grid_structure(value: &Value) -> Result<Option<StructuralSpec>> {
    let is_container = value.get("type").and_then(Value::as_str) == Some("container");

    let is_display_grid = value
        .get("model")
        .and_then(Value::as_str)
        .map(|model| model.starts_with("display-grid-"))
        .unwrap_or(false);

    if !is_container && !is_display_grid {
        return Ok(None);
    }

    let max_children = read_slot_count(value)?;

    let spec = single_relation_spec(
        "A".to_string(),
        "B".to_string(),
        Direction::Positive,
        max_children,
    );

    validate_structural_spec(spec).map(Some)
}

fn single_relation_spec(
    root_level: String,
    child_level: String,
    direction: Direction,
    max_children: usize,
) -> StructuralSpec {
    StructuralSpec {
        version: 1,
        root_level: root_level.clone(),
        relations: vec![StructuralRelation {
            parent_level: root_level,
            child_level,
            direction,
            max_children,
        }],
    }
}

fn validate_structural_spec(spec: StructuralSpec) -> Result<StructuralSpec> {
    if spec.version != 1 {
        bail!("unsupported structural spec version: {}", spec.version);
    }

    validate_level("root", &spec.root_level)?;

    if spec.relations.is_empty() {
        bail!("structural spec must contain at least one relation");
    }

    let mut seen_relations = HashSet::new();

    for relation in &spec.relations {
        validate_level("relation parent", &relation.parent_level)?;

        validate_level("relation child", &relation.child_level)?;

        let parent_index = level_index(&relation.parent_level)?;

        let child_index = level_index(&relation.child_level)?;

        let expected_child = parent_index
            .checked_add(1)
            .ok_or_else(|| anyhow!("child level overflow"))?;

        if child_index != expected_child {
            bail!(
                "child level {} must directly follow parent level {}",
                relation.child_level,
                relation.parent_level
            );
        }

        if relation.max_children == 0 {
            bail!(
                "maxChildren must be greater than zero for {} -> {} {}",
                relation.parent_level,
                relation.child_level,
                direction_symbol(relation.direction)
            );
        }

        let key = (
            relation.parent_level.clone(),
            relation.child_level.clone(),
            relation.direction,
        );

        if !seen_relations.insert(key) {
            bail!(
                "duplicate structural relation {} -> {} {}",
                relation.parent_level,
                relation.child_level,
                direction_symbol(relation.direction)
            );
        }
    }

    validate_reachability(&spec)?;

    Ok(spec)
}

fn validate_reachability(spec: &StructuralSpec) -> Result<()> {
    let mut reachable = HashSet::new();
    reachable.insert(spec.root_level.clone());

    loop {
        let mut changed = false;

        for relation in &spec.relations {
            if reachable.contains(&relation.parent_level)
                && reachable.insert(relation.child_level.clone())
            {
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    for relation in &spec.relations {
        if !reachable.contains(&relation.parent_level) || !reachable.contains(&relation.child_level)
        {
            bail!(
                "relation {} -> {} is not reachable from root level {}",
                relation.parent_level,
                relation.child_level,
                spec.root_level
            );
        }
    }

    Ok(())
}

fn validate_level(label: &str, level: &str) -> Result<()> {
    level_index(level)
        .map(|_| ())
        .map_err(|_| anyhow!("{} level must be one uppercase letter from A to J", label))
}

fn level_index(level: &str) -> Result<u8> {
    let bytes = level.as_bytes();

    if bytes.len() != 1 || !(b'A'..=b'J').contains(&bytes[0]) {
        bail!("invalid structural level: {}", level);
    }

    Ok(bytes[0] - b'A')
}

fn read_slot_count(value: &Value) -> Result<usize> {
    let slot_count = value
        .get("slotCount")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("structural spec has no slotCount"))?;

    let slot_count = usize::try_from(slot_count).map_err(|_| anyhow!("slotCount is too large"))?;

    if slot_count == 0 {
        bail!("slotCount must be greater than zero");
    }

    Ok(slot_count)
}

fn read_direction(value: &Value, label: &str) -> Result<Direction> {
    serde_json::from_value(value.clone()).map_err(|error| anyhow!("invalid {}: {}", label, error))
}

fn direction_symbol(direction: Direction) -> &'static str {
    match direction {
        Direction::Positive => "+",
        Direction::Negative => "-",
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyCollectorSpec {
    compose: LegacyComposeStructure,

    #[serde(default)]
    hierarchy: Option<LegacyHierarchyStructure>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyComposeStructure {
    root: LegacyRootStructure,
    children: LegacyChildrenStructure,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyRootStructure {
    level: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyChildrenStructure {
    #[serde(rename = "type")]
    range_type: String,

    side: Direction,
    level: String,

    #[serde(default)]
    max_items: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyHierarchyStructure {
    root_level: String,
    child_level: String,
    direction: Direction,

    #[serde(default)]
    max_groups: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canonical_suitcase_spec() -> Value {
        json!({
            "ordifi": "1.0",
            "kind": "suitcase",

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
            },

            "render": {
                "component": "storageGrid",
                "x": 100,
                "y": 200
            }
        })
    }

    #[test]
    fn reads_canonical_structural_dictionary() {
        let result = read_structural_spec(&canonical_suitcase_spec()).unwrap();

        assert_eq!(result.version, 1);
        assert_eq!(result.root_level, "A");
        assert_eq!(result.relations.len(), 1);

        let relation = &result.relations[0];

        assert_eq!(relation.parent_level, "A");
        assert_eq!(relation.child_level, "B");
        assert_eq!(relation.direction, Direction::Positive);
        assert_eq!(relation.max_children, 48);
    }

    #[test]
    fn supports_two_directional_spaces() {
        let value = json!({
            "structure": {
                "version": 1,
                "rootLevel": "A",
                "relations": [
                    {
                        "parentLevel": "A",
                        "childLevel": "B",
                        "direction": "-",
                        "maxChildren": 10
                    },
                    {
                        "parentLevel": "A",
                        "childLevel": "B",
                        "direction": "+",
                        "maxChildren": 20
                    }
                ]
            }
        });

        let result = read_structural_spec(&value).unwrap();

        assert!(result.relation("A", "B", Direction::Negative,).is_some());

        assert!(result.relation("A", "B", Direction::Positive,).is_some());
    }

    #[test]
    fn ignores_visual_fields() {
        let mut value = canonical_suitcase_spec();

        value["render"]["x"] = json!(999999);

        value["layout"] = json!({
            "width": 2048,
            "height": 2048
        });

        let result = read_structural_spec(&value).unwrap();

        assert_eq!(result.relations[0].max_children, 48);
    }

    #[test]
    fn reads_legacy_suitcase_dictionary() {
        let value = json!({
            "compose": {
                "root": {
                    "level": "A",
                    "offset": 0
                },
                "children": {
                    "type": "offset-range",
                    "side": "+",
                    "level": "B",
                    "maxItems": 48
                }
            },
            "hierarchy": {
                "rootLevel": "A",
                "childLevel": "B",
                "direction": "+",
                "maxGroups": 48
            }
        });

        let result = read_structural_spec(&value).unwrap();

        assert_eq!(result.root_level, "A");
        assert_eq!(result.relations[0].max_children, 48);
    }

    #[test]
    fn reads_legacy_case_dictionary() {
        let value = json!({
            "kind": "layout",
            "layoutName": "case",
            "compose": {
                "role": "append-layout",
                "displayedContent": {
                    "type": "offset",
                    "direction": "-",
                    "offset": 0
                }
            },
            "slotCount": 1
        });

        let result = read_structural_spec(&value).unwrap();

        assert_eq!(result.relations[0].direction, Direction::Negative);

        assert_eq!(result.relations[0].max_children, 1);
    }

    #[test]
    fn reads_legacy_grid_dictionary() {
        let value = json!({
            "kind": "layout",
            "type": "container",
            "model": "display-grid-4",
            "slotCount": 4
        });

        let result = read_structural_spec(&value).unwrap();

        assert_eq!(result.relations[0].direction, Direction::Positive);

        assert_eq!(result.relations[0].max_children, 4);
    }

    #[test]
    fn rejects_duplicate_relation() {
        let value = json!({
            "structure": {
                "version": 1,
                "rootLevel": "A",
                "relations": [
                    {
                        "parentLevel": "A",
                        "childLevel": "B",
                        "direction": "+",
                        "maxChildren": 4
                    },
                    {
                        "parentLevel": "A",
                        "childLevel": "B",
                        "direction": "+",
                        "maxChildren": 9
                    }
                ]
            }
        });

        let error = read_structural_spec(&value).expect_err("duplicate relation must fail");

        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn rejects_skipped_level() {
        let value = json!({
            "structure": {
                "version": 1,
                "rootLevel": "A",
                "relations": [
                    {
                        "parentLevel": "A",
                        "childLevel": "C",
                        "direction": "+",
                        "maxChildren": 4
                    }
                ]
            }
        });

        let error = read_structural_spec(&value).expect_err("skipped level must fail");

        assert!(error.to_string().contains("directly follow"));
    }
}
