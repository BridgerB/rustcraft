//! Block ↔ state-id conversion. Port of typecraft's `block.ts`.

use std::collections::HashMap;

use crate::chunk::ChunkColumn;
use crate::registry::Registry;

/// A block's name and state properties.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInfo {
    pub name: String,
    pub properties: HashMap<String, String>,
}

/// Convert a block state id to a block name + properties.
pub fn state_id_to_block(registry: &Registry, state_id: u32) -> BlockInfo {
    let block = registry
        .blocks_by_state_id
        .get(&state_id)
        .unwrap_or_else(|| panic!("Unknown state ID: {state_id}"));

    if block.states.is_empty() || state_id == block.default_state {
        return BlockInfo {
            name: block.name.clone(),
            properties: HashMap::new(),
        };
    }

    let mut properties = HashMap::new();
    let mut remaining = state_id - block.min_state_id;
    let mut multiplier: u32 = block.states.iter().map(|s| s.num_values).product();

    for prop in &block.states {
        multiplier /= prop.num_values;
        let idx = (remaining / multiplier) as usize;
        remaining %= multiplier;
        if let Some(values) = &prop.values {
            let value = values.get(idx).cloned().unwrap_or_else(|| idx.to_string());
            properties.insert(prop.name.clone(), value);
        }
    }

    BlockInfo {
        name: block.name.clone(),
        properties,
    }
}

/// Convert a block name + optional properties to a block state id.
pub fn block_to_state_id(
    registry: &Registry,
    name: &str,
    properties: Option<&HashMap<String, String>>,
) -> u32 {
    let block = registry
        .blocks_by_name
        .get(name)
        .unwrap_or_else(|| panic!("Unknown block: {name}"));

    let props_empty = properties.map(|p| p.is_empty()).unwrap_or(true);
    if props_empty || block.states.is_empty() {
        return block.default_state;
    }
    let properties = properties.unwrap();

    let mut offset = 0u32;
    let mut multiplier = 1u32;
    for prop in block.states.iter().rev() {
        if let (Some(prop_value), Some(values)) = (properties.get(&prop.name), &prop.values) {
            if let Some(idx) = values.iter().position(|v| v == prop_value) {
                offset += idx as u32 * multiplier;
            }
        }
        multiplier *= prop.num_values;
    }

    block.min_state_id + offset
}

/// Get the block at a position as a name + properties.
pub fn get_block(col: &ChunkColumn, x: i32, y: i32, z: i32, registry: &Registry) -> BlockInfo {
    state_id_to_block(registry, col.get_block_state_id(x, y, z))
}

/// Set a block at a position by name and optional properties.
pub fn set_block(
    col: &mut ChunkColumn,
    x: i32,
    y: i32,
    z: i32,
    registry: &Registry,
    name: &str,
    properties: Option<&HashMap<String, String>>,
) {
    col.set_block_state_id(x, y, z, block_to_state_id(registry, name, properties));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{BlockCollisionShapes, BlockDefinition, BlockStateProperty, Registry};

    fn facing_block() -> BlockDefinition {
        // A block with 4 facing values × 2 powered values = 8 states (100..=107).
        BlockDefinition {
            id: 10,
            name: "lever".into(),
            display_name: "Lever".into(),
            hardness: Some(0.5),
            resistance: Some(0.5),
            stack_size: 64,
            diggable: true,
            bounding_box: "empty".into(),
            material: None,
            transparent: true,
            emit_light: 0,
            filter_light: 0,
            default_state: 100,
            min_state_id: 100,
            max_state_id: 107,
            states: vec![
                BlockStateProperty {
                    name: "facing".into(),
                    ty: "enum".into(),
                    num_values: 4,
                    values: Some(vec![
                        "north".into(),
                        "south".into(),
                        "west".into(),
                        "east".into(),
                    ]),
                },
                BlockStateProperty {
                    name: "powered".into(),
                    ty: "bool".into(),
                    num_values: 2,
                    values: Some(vec!["true".into(), "false".into()]),
                },
            ],
            drops: vec![],
            harvest_tools: None,
        }
    }

    fn registry() -> Registry {
        Registry::build(
            vec![
                BlockDefinition {
                    id: 0,
                    name: "air".into(),
                    display_name: "Air".into(),
                    hardness: None,
                    resistance: None,
                    stack_size: 64,
                    diggable: true,
                    bounding_box: "empty".into(),
                    material: None,
                    transparent: true,
                    emit_light: 0,
                    filter_light: 0,
                    default_state: 0,
                    min_state_id: 0,
                    max_state_id: 0,
                    states: vec![],
                    drops: vec![],
                    harvest_tools: None,
                },
                facing_block(),
            ],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            BlockCollisionShapes::default(),
            std::collections::HashMap::new(),
            "26.1.2",
        )
    }

    #[test]
    fn default_state_has_no_properties() {
        let reg = registry();
        let info = state_id_to_block(&reg, 100);
        assert_eq!(info.name, "lever");
        assert!(info.properties.is_empty());
    }

    #[test]
    fn decodes_state_properties() {
        let reg = registry();
        // state 103 = (103-100)=3 → facing idx = 3/2 = 1 (south), powered idx = 3%2 = 1 (false)
        let info = state_id_to_block(&reg, 103);
        assert_eq!(info.properties.get("facing").unwrap(), "south");
        assert_eq!(info.properties.get("powered").unwrap(), "false");
    }

    #[test]
    fn roundtrips_state_id() {
        let reg = registry();
        for state_id in 100..=107 {
            let info = state_id_to_block(&reg, state_id);
            let back = block_to_state_id(&reg, &info.name, Some(&info.properties));
            assert_eq!(back, state_id, "roundtrip failed for state {state_id}");
        }
    }

    #[test]
    fn block_to_state_id_default_without_properties() {
        let reg = registry();
        assert_eq!(block_to_state_id(&reg, "lever", None), 100);
        assert_eq!(block_to_state_id(&reg, "air", None), 0);
    }
}
