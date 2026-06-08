//! Minecraft data-registry types, deserialized from the JSON produced by
//! typecraft's datagen (`src/data/`). Field names match the prismarine-data
//! conventions used by that output.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDefinition {
    pub id: i32,
    pub name: String,
    pub display_name: String,
    pub stack_size: i32,
    #[serde(default)]
    pub enchant_categories: Option<Vec<String>>,
    #[serde(default)]
    pub repair_with: Option<Vec<String>>,
    #[serde(default)]
    pub max_durability: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockStateProperty {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub num_values: u32,
    #[serde(default)]
    pub values: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockDefinition {
    pub id: i32,
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub hardness: Option<f64>,
    #[serde(default)]
    pub resistance: Option<f64>,
    pub stack_size: i32,
    pub diggable: bool,
    pub bounding_box: String,
    #[serde(default)]
    pub material: Option<String>,
    pub transparent: bool,
    pub emit_light: i32,
    pub filter_light: i32,
    pub default_state: u32,
    pub min_state_id: u32,
    pub max_state_id: u32,
    #[serde(default)]
    pub states: Vec<BlockStateProperty>,
    #[serde(default)]
    pub drops: Vec<serde_json::Value>,
    #[serde(default)]
    pub harvest_tools: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityDefinition {
    pub id: i32,
    pub name: String,
    pub display_name: String,
    pub width: f64,
    pub height: f64,
    #[serde(rename = "type")]
    pub ty: String,
    pub category: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectDefinition {
    pub id: i32,
    pub name: String,
    pub display_name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttributeDefinition {
    pub resource: String,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

#[derive(Debug, Clone)]
pub struct BiomeDefinition {
    pub id: i32,
    pub name: String,
    pub display_name: String,
    pub category: String,
    pub temperature: f64,
    pub dimension: String,
    pub color: i32,
    pub rainfall: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct EnchantmentDefinition {
    pub id: i32,
    pub name: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct FoodDefinition {
    pub id: i32,
    pub name: String,
    pub display_name: String,
}

/// A recipe ingredient: absent, a concrete item id, or an item id carrying
/// optional metadata and a set of acceptable substitutes (`choices`, from a tag).
#[derive(Debug, Clone, PartialEq)]
pub enum RawRecipeItem {
    None,
    Id(i32),
    Detailed {
        id: i32,
        metadata: Option<i32>,
        choices: Option<Vec<i32>>,
    },
}

#[derive(Debug, Clone)]
pub struct RecipeResult {
    pub id: i32,
    pub count: i32,
    pub metadata: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct RawRecipe {
    pub in_shape: Option<Vec<Vec<RawRecipeItem>>>,
    pub out_shape: Option<Vec<Vec<RawRecipeItem>>>,
    pub ingredients: Option<Vec<RawRecipeItem>>,
    pub result: RecipeResult,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BlockCollisionShapes {
    #[serde(default)]
    pub blocks: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub shapes: HashMap<String, Vec<Vec<f64>>>,
}

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub kind: &'static str,
    pub major_version: String,
    pub minecraft_version: String,
    pub version: i32,
    pub data_version: i32,
}

/// A version-gated feature value (mirrors prismarine `features.json`): most are
/// booleans, a few are strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureValue {
    Bool(bool),
    Str(&'static str),
}

impl FeatureValue {
    pub fn as_bool(self) -> bool {
        matches!(self, FeatureValue::Bool(true))
    }

    pub fn as_str(self) -> Option<&'static str> {
        match self {
            FeatureValue::Str(s) => Some(s),
            _ => None,
        }
    }
}
