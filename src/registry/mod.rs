//! The loaded Minecraft data registry for a specific version: indexed
//! definitions, recipes, collision shapes, and version-gated feature flags.
//! Port of typecraft's `registry` module.

mod types;

pub use types::*;

use std::collections::HashMap;
use std::path::Path;

pub struct Registry {
    pub version: VersionInfo,

    pub blocks_by_id: HashMap<i32, BlockDefinition>,
    pub blocks_by_name: HashMap<String, BlockDefinition>,
    pub blocks_by_state_id: HashMap<u32, BlockDefinition>,
    pub blocks_array: Vec<BlockDefinition>,

    pub biomes_by_id: HashMap<i32, BiomeDefinition>,
    pub biomes_by_name: HashMap<String, BiomeDefinition>,
    pub biomes_array: Vec<BiomeDefinition>,

    pub items_by_id: HashMap<i32, ItemDefinition>,
    pub items_by_name: HashMap<String, ItemDefinition>,
    pub items_array: Vec<ItemDefinition>,

    pub entities_by_id: HashMap<i32, EntityDefinition>,
    pub entities_by_name: HashMap<String, EntityDefinition>,
    pub entities_array: Vec<EntityDefinition>,

    pub enchantments_by_id: HashMap<i32, EnchantmentDefinition>,
    pub enchantments_by_name: HashMap<String, EnchantmentDefinition>,
    pub enchantments_array: Vec<EnchantmentDefinition>,

    pub foods_by_id: HashMap<i32, FoodDefinition>,
    pub foods_by_name: HashMap<String, FoodDefinition>,
    pub foods_array: Vec<FoodDefinition>,

    pub effects_by_id: HashMap<i32, EffectDefinition>,
    pub effects_by_name: HashMap<String, EffectDefinition>,
    pub effects_array: Vec<EffectDefinition>,

    pub attributes_by_name: HashMap<String, AttributeDefinition>,
    pub attributes_array: Vec<AttributeDefinition>,

    pub block_collision_shapes: BlockCollisionShapes,
    /// Recipes keyed by result item id.
    pub recipes: HashMap<i32, Vec<RawRecipe>>,

    data_version: i32,
    features: HashMap<&'static str, FeatureValue>,
}

const VERSION_LOOKUP: &[(&str, i32)] = &[
    ("1.8", 100),
    ("1.9", 169),
    ("1.10", 510),
    ("1.11", 819),
    ("1.12", 1139),
    ("1.13", 1519),
    ("1.14", 1901),
    ("1.15", 2225),
    ("1.16", 2566),
    ("1.17", 2724),
    ("1.18", 2860),
    ("1.19", 3105),
    ("1.20", 3463),
    ("1.20.4", 3700),
    ("1.20.5", 3837),
    ("1.21", 3953),
    ("1.21.1", 3955),
    ("1.21.11", 4384),
    ("26.1.2", 4900),
];

fn version_data(v: &str) -> Option<i32> {
    VERSION_LOOKUP
        .iter()
        .find(|(k, _)| *k == v)
        .map(|(_, n)| *n)
}

impl Registry {
    pub fn is_newer_or_equal_to(&self, v: &str) -> bool {
        self.data_version >= version_data(v).unwrap_or(0)
    }

    pub fn is_older_than(&self, v: &str) -> bool {
        self.data_version < version_data(v).unwrap_or(i32::MAX)
    }

    pub fn support_feature(&self, feature: &str) -> FeatureValue {
        self.features
            .get(feature)
            .copied()
            .unwrap_or(FeatureValue::Bool(false))
    }

    /// Build a registry from already-loaded definitions (pure; no file IO).
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        blocks_array: Vec<BlockDefinition>,
        items_array: Vec<ItemDefinition>,
        entities_array: Vec<EntityDefinition>,
        effects_array: Vec<EffectDefinition>,
        attributes_array: Vec<AttributeDefinition>,
        biomes_array: Vec<BiomeDefinition>,
        block_collision_shapes: BlockCollisionShapes,
        recipes: HashMap<i32, Vec<RawRecipe>>,
        version: &str,
    ) -> Registry {
        let mut blocks_by_id = HashMap::new();
        let mut blocks_by_name = HashMap::new();
        let mut blocks_by_state_id = HashMap::new();
        for block in &blocks_array {
            blocks_by_id.insert(block.id, block.clone());
            blocks_by_name.insert(block.name.clone(), block.clone());
            for s in block.min_state_id..=block.max_state_id {
                blocks_by_state_id.insert(s, block.clone());
            }
        }

        // Each definition type with an `id` and `name` builds the same by_id/by_name
        // index pair; generate them rather than repeat the loop verbatim. (blocks adds a
        // by_state_id pass and attributes are name-only, so those stay hand-written.)
        macro_rules! index_id_name {
            ($array:expr, $by_id:ident, $by_name:ident) => {
                let mut $by_id = HashMap::new();
                let mut $by_name = HashMap::new();
                for def in &$array {
                    $by_id.insert(def.id, def.clone());
                    $by_name.insert(def.name.clone(), def.clone());
                }
            };
        }
        index_id_name!(biomes_array, biomes_by_id, biomes_by_name);
        index_id_name!(items_array, items_by_id, items_by_name);
        index_id_name!(entities_array, entities_by_id, entities_by_name);
        index_id_name!(effects_array, effects_by_id, effects_by_name);

        let mut attributes_by_name = HashMap::new();
        for attr in &attributes_array {
            attributes_by_name.insert(attr.name.clone(), attr.clone());
        }

        let parts: Vec<&str> = version.split('.').collect();
        let major = format!(
            "{}.{}",
            parts.first().copied().unwrap_or("1"),
            parts.get(1).copied().unwrap_or("21")
        );

        // Target: Minecraft 26.1.2, protocol 775 (matches the embedded protocol
        // schema). data_version is the 26.1.2 lineage value.
        let data_version = version_data(version).unwrap_or(4900);
        let version_info = VersionInfo {
            kind: "pc",
            major_version: major,
            minecraft_version: version.to_string(),
            version: 775,
            data_version,
        };

        let features = build_features(data_version);

        Registry {
            version: version_info,
            blocks_by_id,
            blocks_by_name,
            blocks_by_state_id,
            blocks_array,
            biomes_by_id,
            biomes_by_name,
            biomes_array,
            items_by_id,
            items_by_name,
            items_array,
            entities_by_id,
            entities_by_name,
            entities_array,
            enchantments_by_id: HashMap::new(),
            enchantments_by_name: HashMap::new(),
            enchantments_array: Vec::new(),
            foods_by_id: HashMap::new(),
            foods_by_name: HashMap::new(),
            foods_array: Vec::new(),
            effects_by_id,
            effects_by_name,
            effects_array,
            attributes_by_name,
            attributes_array,
            block_collision_shapes,
            recipes,
            data_version,
            features,
        }
    }
}

fn build_features(data_version: i32) -> HashMap<&'static str, FeatureValue> {
    let newer = |v: &str| data_version >= version_data(v).unwrap_or(0);
    let older = |v: &str| data_version < version_data(v).unwrap_or(i32::MAX);
    let b = FeatureValue::Bool;

    // Boolean version-gated features: (name, condition). Built once from a table so
    // adding a flag is a one-line edit. String-valued features (which pick between two
    // identifiers) follow, since they don't fit the bool shape.
    let bools: [(&'static str, bool); 33] = [
        ("village&pillageInventoryWindows", newer("1.14")),
        ("netherUpdateInventoryWindows", newer("1.16")),
        ("shieldSlot", newer("1.9")),
        ("itemSerializationUsesBlockId", older("1.13")),
        ("booksUseStoredEnchantments", true),
        ("spawnEggsHaveSpawnedEntityInName", newer("1.13")),
        ("spawnEggsUseEntityTagInNbt", older("1.13")),
        ("fixedPointPosition", older("1.9")),
        ("fixedPointDelta128", older("1.9")),
        ("entityVelocityIsLpVec3", newer("1.21")),
        ("playerInfoActionIsBitfield", newer("1.19")),
        ("armAnimationBeforeUse", newer("1.9")),
        ("newPlayerInputPacket", newer("1.21")),
        ("entityActionUsesStringMapper", newer("1.21")),
        ("spawnRespawnWorldDataField", newer("1.21")),
        ("dimensionIsAnInt", older("1.16")),
        ("dimensionIsAString", newer("1.16") && older("1.19")),
        ("dimensionIsAWorld", newer("1.19")),
        ("segmentedRegistryCodecData", newer("1.20.5")),
        ("customChannelMCPrefixed", newer("1.13")),
        ("stateIdUsed", newer("1.17")),
        ("useItemWithOwnPacket", newer("1.9")),
        ("usesBlockStates", newer("1.13")),
        ("usesMultiblockSingleLong", newer("1.16")),
        ("blockPlaceHasInsideBlock", newer("1.19")),
        ("blockPlaceHasHandAndFloatCursor", newer("1.9") && older("1.19")),
        ("blockPlaceHasHandAndIntCursor", older("1.9")),
        ("chatPacketsUseNbtComponents", newer("1.20")),
        ("independentLiquidGravity", newer("1.13")),
        ("velocityBlocksOnTop", newer("1.9")),
        ("climbUsingJump", older("1.14")),
        ("climbableTrapdoor", newer("1.9")),
        ("respawnIsPayload", newer("1.20")),
    ];
    let mut f: HashMap<&'static str, FeatureValue> = HashMap::new();
    for (name, cond) in bools {
        f.insert(name, b(cond));
    }
    let str_feat =
        |cond: bool, yes: &'static str, no: &'static str| FeatureValue::Str(if cond { yes } else { no });
    f.insert("nbtNameForEnchant", str_feat(newer("1.13"), "Enchantments", "ench"));
    f.insert("typeOfValueForEnchantLevel", str_feat(newer("1.13"), "string", "short"));
    f.insert("whereDurabilityIsSerialized", str_feat(newer("1.13"), "Damage", "metadata"));
    f
}

// ── File loading ──

fn load_json<T: serde::de::DeserializeOwned>(dir: &Path, file: &str) -> std::io::Result<T> {
    let text = std::fs::read_to_string(dir.join(file))?;
    serde_json::from_str(&text).map_err(std::io::Error::other)
}

#[derive(serde::Deserialize)]
struct RawBiome {
    temperature: Option<f64>,
    downfall: Option<f64>,
}

/// Load a registry from a data directory (the datagen output, `src/data/`).
pub fn create_registry(data_dir: impl AsRef<Path>, version: &str) -> std::io::Result<Registry> {
    let dir = data_dir.as_ref();

    let blocks: Vec<BlockDefinition> = load_json(dir, "blocks.json")?;
    let items: Vec<ItemDefinition> = load_json(dir, "items.json")?;
    let entities: Vec<EntityDefinition> = load_json(dir, "entities.json")?;
    let effects: Vec<EffectDefinition> = load_json(dir, "effects.json")?;
    let attributes: Vec<AttributeDefinition> = load_json(dir, "attributes.json")?;
    let block_collision_shapes: BlockCollisionShapes =
        load_json(dir, "blockCollisionShapes.json").unwrap_or_default();

    let mut biomes = Vec::new();
    let biomes_dir = dir.join("biomes-raw");
    if biomes_dir.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(&biomes_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".json"))
            .collect();
        files.sort();
        for (i, file) in files.iter().enumerate() {
            let name = file.trim_end_matches(".json").to_string();
            let raw: RawBiome = load_json(&biomes_dir, file)?;
            biomes.push(BiomeDefinition {
                id: i as i32,
                name: name.clone(),
                display_name: name,
                category: "none".into(),
                temperature: raw.temperature.unwrap_or(0.5),
                dimension: "overworld".into(),
                color: 0,
                rainfall: raw.downfall,
            });
        }
    }

    let item_ids: HashMap<String, i32> = items.iter().map(|i| (i.name.clone(), i.id)).collect();
    let recipes = load_recipes(dir, &item_ids);

    Ok(Registry::build(
        blocks,
        items,
        entities,
        effects,
        attributes,
        biomes,
        block_collision_shapes,
        recipes,
        version,
    ))
}

// ── Recipe loading ──

#[derive(serde::Deserialize)]
struct RawTagFile {
    values: Vec<String>,
}

#[derive(serde::Deserialize)]
struct RawVanillaRecipe {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    key: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pattern: Vec<String>,
    #[serde(default)]
    ingredients: Vec<serde_json::Value>,
    result: RawResult,
}

#[derive(serde::Deserialize)]
struct RawResult {
    id: String,
    #[serde(default)]
    count: Option<i32>,
}

fn load_item_tags(dir: &Path) -> HashMap<String, Vec<String>> {
    let tags_dir = dir.join("tags/item");
    if !tags_dir.is_dir() {
        return HashMap::new();
    }

    let mut raw: HashMap<String, Vec<String>> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&tags_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let fname = entry.file_name().to_string_lossy().into_owned();
            if !fname.ends_with(".json") {
                continue;
            }
            let name = fname.trim_end_matches(".json").to_string();
            if let Ok(data) = load_json::<RawTagFile>(&tags_dir, &fname) {
                raw.insert(
                    name,
                    data.values
                        .into_iter()
                        .map(|v| v.replace("minecraft:", ""))
                        .collect(),
                );
            }
        }
    }

    fn resolve(
        name: &str,
        raw: &HashMap<String, Vec<String>>,
        seen: &mut Vec<String>,
    ) -> Vec<String> {
        if seen.iter().any(|s| s == name) {
            return Vec::new();
        }
        seen.push(name.to_string());
        let Some(values) = raw.get(name) else {
            return Vec::new();
        };
        let mut resolved = Vec::new();
        for v in values {
            if let Some(tag) = v.strip_prefix('#') {
                resolved.extend(resolve(tag, raw, seen));
            } else {
                resolved.push(v.clone());
            }
        }
        resolved
    }

    let mut tags = HashMap::new();
    for name in raw.keys() {
        let mut seen = Vec::new();
        tags.insert(name.clone(), resolve(name, &raw, &mut seen));
    }
    tags
}

/// Resolve a recipe ingredient reference to a `RawRecipeItem`.
fn resolve_ingredient(
    reference: &str,
    items_by_name: &HashMap<String, i32>,
    tags: &HashMap<String, Vec<String>>,
) -> RawRecipeItem {
    if let Some(tag_name) = reference.strip_prefix("#minecraft:") {
        let Some(tag_items) = tags.get(tag_name) else {
            return RawRecipeItem::None;
        };
        let ids: Vec<i32> = tag_items
            .iter()
            .filter_map(|n| items_by_name.get(n).copied())
            .collect();
        return match ids.len() {
            0 => RawRecipeItem::None,
            1 => RawRecipeItem::Id(ids[0]),
            _ => RawRecipeItem::Detailed {
                id: ids[0],
                metadata: None,
                choices: Some(ids),
            },
        };
    }
    let name = reference.replace("minecraft:", "");
    match items_by_name.get(&name) {
        Some(&id) => RawRecipeItem::Id(id),
        None => RawRecipeItem::None,
    }
}

/// First string in a JSON ingredient value (`"x"` or `["x", ...]`).
fn ingredient_ref(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) => a.first().and_then(|v| v.as_str()).map(String::from),
        _ => None,
    }
}

fn load_recipes(dir: &Path, items_by_name: &HashMap<String, i32>) -> HashMap<i32, Vec<RawRecipe>> {
    let recipes_dir = dir.join("recipes-raw");
    if !recipes_dir.is_dir() {
        return HashMap::new();
    }
    let tags = load_item_tags(dir);
    let mut by_result: HashMap<i32, Vec<RawRecipe>> = HashMap::new();

    let Ok(entries) = std::fs::read_dir(&recipes_dir) else {
        return by_result;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let fname = entry.file_name().to_string_lossy().into_owned();
        if !fname.ends_with(".json") {
            continue;
        }
        let Ok(raw) = load_json::<RawVanillaRecipe>(&recipes_dir, &fname) else {
            continue;
        };

        let result_name = raw.result.id.replace("minecraft:", "");
        let Some(&result_id) = items_by_name.get(&result_name) else {
            continue;
        };
        let result = RecipeResult {
            id: result_id,
            count: raw.result.count.unwrap_or(1),
            metadata: None,
        };

        if raw.ty == "minecraft:crafting_shaped" && !raw.pattern.is_empty() && !raw.key.is_empty() {
            let mut in_shape = Vec::new();
            for row in &raw.pattern {
                let mut shape_row = Vec::new();
                for ch in row.chars() {
                    if ch == ' ' {
                        shape_row.push(RawRecipeItem::None);
                        continue;
                    }
                    match raw.key.get(&ch.to_string()).and_then(ingredient_ref) {
                        Some(reference) => {
                            shape_row.push(resolve_ingredient(&reference, items_by_name, &tags))
                        }
                        None => shape_row.push(RawRecipeItem::None),
                    }
                }
                in_shape.push(shape_row);
            }
            by_result.entry(result.id).or_default().push(RawRecipe {
                in_shape: Some(in_shape),
                out_shape: None,
                ingredients: None,
                result,
            });
        } else if raw.ty == "minecraft:crafting_shapeless" && !raw.ingredients.is_empty() {
            let mut ingredients = Vec::new();
            for ing in &raw.ingredients {
                if let Some(reference) = ingredient_ref(ing) {
                    ingredients.push(resolve_ingredient(&reference, items_by_name, &tags));
                }
            }
            by_result.entry(result.id).or_default().push(RawRecipe {
                in_shape: None,
                out_shape: None,
                ingredients: Some(ingredients),
                result,
            });
        }
    }

    by_result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(
        name: &str,
        id: i32,
        min: u32,
        max: u32,
        states: Vec<BlockStateProperty>,
    ) -> BlockDefinition {
        BlockDefinition {
            id,
            name: name.into(),
            display_name: name.into(),
            hardness: Some(1.0),
            resistance: Some(1.0),
            stack_size: 64,
            diggable: true,
            bounding_box: "block".into(),
            material: None,
            transparent: false,
            emit_light: 0,
            filter_light: 15,
            default_state: min,
            min_state_id: min,
            max_state_id: max,
            states,
            drops: vec![],
            harvest_tools: None,
        }
    }

    fn test_registry() -> Registry {
        let blocks = vec![
            block("air", 0, 0, 0, vec![]),
            block("stone", 1, 1, 1, vec![]),
        ];
        Registry::build(
            blocks,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            BlockCollisionShapes::default(),
            HashMap::new(),
            "26.1.2",
        )
    }

    #[test]
    fn indexes_blocks() {
        let reg = test_registry();
        assert_eq!(reg.blocks_by_name.get("stone").unwrap().id, 1);
        assert_eq!(reg.blocks_by_id.get(&1).unwrap().name, "stone");
        assert_eq!(reg.blocks_by_state_id.get(&1).unwrap().name, "stone");
    }

    #[test]
    fn version_comparisons() {
        let reg = test_registry();
        assert!(reg.is_newer_or_equal_to("1.21"));
        assert!(reg.is_newer_or_equal_to("1.13"));
        assert!(!reg.is_older_than("1.21"));
        assert!(reg.is_older_than("1.99"));
    }

    #[test]
    fn feature_flags() {
        let reg = test_registry();
        assert!(reg.support_feature("stateIdUsed").as_bool());
        assert!(reg.support_feature("usesBlockStates").as_bool());
        assert!(!reg.support_feature("dimensionIsAnInt").as_bool());
        assert_eq!(
            reg.support_feature("nbtNameForEnchant").as_str(),
            Some("Enchantments")
        );
        assert!(!reg.support_feature("nonexistentFeature").as_bool());
    }

    #[test]
    fn deserializes_block_json() {
        let json = r#"[{
            "id": 1, "name": "stone", "displayName": "Stone",
            "hardness": 1.5, "resistance": 6.0, "stackSize": 64,
            "diggable": true, "boundingBox": "block", "transparent": false,
            "emitLight": 0, "filterLight": 15,
            "defaultState": 1, "minStateId": 1, "maxStateId": 1,
            "states": [], "drops": []
        }]"#;
        let blocks: Vec<BlockDefinition> = serde_json::from_str(json).unwrap();
        assert_eq!(blocks[0].name, "stone");
        assert_eq!(blocks[0].hardness, Some(1.5));
    }
}
