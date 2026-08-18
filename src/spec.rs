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
 * Reads the canonical structural language.
 *
 * Application, presentation, and other unrelated fields are ignored.
 * BCE does not infer structure from application-specific metadata.
 */
pub fn read_structural_spec(value: &Value) -> Result<StructuralSpec> {
    let structure = value
        .get("structure")
        .ok_or_else(|| anyhow!("spec contains no canonical structure"))?;

    read_canonical_structure(structure)
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

fn direction_symbol(direction: Direction) -> &'static str {
    match direction {
        Direction::Positive => "+",
        Direction::Negative => "-",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn canonical_spec() -> Value {
        json!({
            "application": {
                "name": "example"
            },

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

            "presentation": {
                "component": "example",
                "x": 100,
                "y": 200
            }
        })
    }

    #[test]
    fn reads_canonical_structural_dictionary() {
        let result = read_structural_spec(&canonical_spec()).unwrap();

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

        assert!(result.relation("A", "B", Direction::Negative).is_some());
        assert!(result.relation("A", "B", Direction::Positive).is_some());
    }

    #[test]
    fn ignores_unrelated_fields() {
        let mut value = canonical_spec();

        value["presentation"]["x"] = json!(999999);

        value["anythingElse"] = json!({
            "width": 2048,
            "height": 2048
        });

        let result = read_structural_spec(&value).unwrap();

        assert_eq!(result.relations[0].max_children, 48);
    }

    #[test]
    fn rejects_missing_canonical_structure() {
        let value = json!({
            "application": {
                "name": "example"
            },
            "direction": "-",
            "slotCount": 48
        });

        let error =
            read_structural_spec(&value).expect_err("missing canonical structure must fail");

        assert!(error.to_string().contains("no canonical structure"));
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
