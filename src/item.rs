//! Item stacks: creation, inspection (enchants, durability, lore), equality,
//! and network (Notch) (de)serialization. Port of typecraft's `item` module.

use crate::nbt::{nbt_int, nbt_list, nbt_string, NbtCompound, NbtTag, NbtType};
use crate::protocol::{hash_component_data, PValue};
use crate::registry::Registry;

/// A structured component attached to a 1.21+ item.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemComponent {
    pub ty: String,
    pub data: PValue,
    pub hash: i32,
}

/// An enchantment applied to an item.
#[derive(Debug, Clone, PartialEq)]
pub struct Enchant {
    pub name: String,
    pub level: i32,
}

/// A Minecraft item stack.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub type_id: i32,
    pub count: i32,
    pub metadata: i32,
    pub nbt: Option<NbtCompound>,
    pub name: String,
    pub display_name: String,
    pub stack_size: i32,
    pub max_durability: Option<i32>,
    pub components: Vec<ItemComponent>,
    pub removed_components: Vec<String>,
}

// ── Construction ──

#[allow(clippy::too_many_arguments)]
pub fn create_item(
    registry: &Registry,
    type_id: i32,
    count: i32,
    metadata: i32,
    nbt: Option<NbtCompound>,
    components: Vec<ItemComponent>,
    removed_components: Vec<String>,
) -> Item {
    let def = registry.items_by_id.get(&type_id);
    Item {
        type_id,
        count,
        metadata,
        nbt,
        name: def
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "unknown".into()),
        display_name: def
            .map(|d| d.display_name.clone())
            .unwrap_or_else(|| "Unknown".into()),
        stack_size: def.map(|d| d.stack_size).unwrap_or(1),
        max_durability: def.and_then(|d| d.max_durability),
        components,
        removed_components,
    }
}

pub fn create_item_by_name(
    registry: &Registry,
    name: &str,
    count: i32,
    metadata: i32,
    nbt: Option<NbtCompound>,
) -> Result<Item, String> {
    let id = registry
        .items_by_name
        .get(name)
        .map(|d| d.id)
        .ok_or_else(|| format!("Unknown item: {name}"))?;
    Ok(create_item(
        registry,
        id,
        count,
        metadata,
        nbt,
        vec![],
        vec![],
    ))
}

// ── NBT helpers ──

fn get_compound_tag<'a>(nbt: Option<&'a NbtCompound>, key: &str) -> Option<&'a NbtCompound> {
    match nbt?.get(key) {
        Some(NbtTag::Compound(c)) => Some(c),
        _ => None,
    }
}

fn get_display_tag(item: &Item) -> Option<&NbtCompound> {
    get_compound_tag(item.nbt.as_ref(), "display")
}

fn with_nbt_value(item: &Item, key: &str, value: NbtTag) -> Item {
    let mut compound = item.nbt.clone().unwrap_or_default();
    compound.insert(key, value);
    Item {
        nbt: Some(compound),
        ..item.clone()
    }
}

fn with_display_value(item: &Item, key: &str, value: NbtTag) -> Item {
    let mut display = get_display_tag(item).cloned().unwrap_or_default();
    display.insert(key, value);
    with_nbt_value(item, "display", NbtTag::Compound(display))
}

// ── Enchantments ──

pub fn get_enchants(registry: &Registry, item: &Item) -> Vec<Enchant> {
    let Some(nbt) = item.nbt.as_ref() else {
        return vec![];
    };
    let enchant_key = registry
        .support_feature("nbtNameForEnchant")
        .as_str()
        .unwrap_or("Enchantments");
    let level_type = registry
        .support_feature("typeOfValueForEnchantLevel")
        .as_str()
        .unwrap_or("string");
    let use_stored = registry
        .support_feature("booksUseStoredEnchantments")
        .as_bool()
        && item.name == "enchanted_book";

    let list_tag = if use_stored {
        nbt.get("StoredEnchantments")
            .or_else(|| nbt.get(enchant_key))
    } else {
        nbt.get(enchant_key)
    };
    let Some(NbtTag::List(list)) = list_tag else {
        return vec![];
    };

    list.items
        .iter()
        .filter_map(|entry| {
            let NbtTag::Compound(c) = entry else {
                return None;
            };
            let level = match c.get("lvl") {
                Some(NbtTag::Short(s)) => *s as i32,
                Some(NbtTag::Int(i)) => *i,
                _ => 0,
            };
            if level_type == "short" && enchant_key == "ench" {
                let numeric_id = match c.get("id") {
                    Some(NbtTag::Short(s)) => *s as i32,
                    Some(NbtTag::Int(i)) => *i,
                    _ => 0,
                };
                let name = registry
                    .enchantments_by_id
                    .get(&numeric_id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| "unknown".into());
                Some(Enchant { name, level })
            } else {
                let id = c.get("id").and_then(NbtTag::as_string).unwrap_or("");
                Some(Enchant {
                    name: id.replace("minecraft:", ""),
                    level,
                })
            }
        })
        .collect()
}

pub fn set_enchants(registry: &Registry, item: &Item, enchants: &[Enchant]) -> Item {
    let enchant_key = registry
        .support_feature("nbtNameForEnchant")
        .as_str()
        .unwrap_or("Enchantments");
    let level_type = registry
        .support_feature("typeOfValueForEnchantLevel")
        .as_str()
        .unwrap_or("string");
    let use_stored = registry
        .support_feature("booksUseStoredEnchantments")
        .as_bool()
        && item.name == "enchanted_book";
    let key = if use_stored {
        "StoredEnchantments"
    } else {
        enchant_key
    };

    if enchants.is_empty() {
        let Some(nbt) = item.nbt.as_ref() else {
            return item.clone();
        };
        let rest: Vec<(String, NbtTag)> = nbt
            .iter()
            .filter(|(k, _)| *k != key)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return Item {
            nbt: if rest.is_empty() {
                None
            } else {
                Some(rest.into_iter().collect())
            },
            ..item.clone()
        };
    }

    let entries: Vec<NbtTag> = enchants
        .iter()
        .map(|e| {
            let id = if level_type == "short" && enchant_key == "ench" {
                NbtTag::Short(
                    registry
                        .enchantments_by_name
                        .get(&e.name)
                        .map(|d| d.id as i16)
                        .unwrap_or(0),
                )
            } else {
                nbt_string(format!("minecraft:{}", e.name))
            };
            NbtTag::Compound(
                vec![
                    ("id".to_string(), id),
                    ("lvl".to_string(), NbtTag::Short(e.level as i16)),
                ]
                .into_iter()
                .collect(),
            )
        })
        .collect();

    with_nbt_value(item, key, nbt_list(NbtType::Compound, entries))
}

// ── Custom name / lore ──

pub fn get_custom_name(item: &Item) -> Option<String> {
    match get_display_tag(item)?.get("Name") {
        Some(NbtTag::String(s)) => Some(s.clone()),
        _ => None,
    }
}

pub fn set_custom_name(item: &Item, name: &str) -> Item {
    with_display_value(item, "Name", nbt_string(name))
}

pub fn get_custom_lore(item: &Item) -> Option<Vec<String>> {
    let display = get_display_tag(item)?;
    match display.get("Lore")? {
        NbtTag::String(s) => Some(vec![s.clone()]),
        NbtTag::List(list) => Some(
            list.items
                .iter()
                .filter_map(|t| t.as_string().map(String::from))
                .collect(),
        ),
        _ => None,
    }
}

pub fn set_custom_lore(item: &Item, lore: &[String]) -> Item {
    let items = lore.iter().map(|s| nbt_string(s.clone())).collect();
    with_display_value(item, "Lore", nbt_list(NbtType::String, items))
}

// ── Durability / repair ──

pub fn get_durability_used(registry: &Registry, item: &Item) -> Option<i32> {
    item.max_durability?;
    let where_dur = registry
        .support_feature("whereDurabilityIsSerialized")
        .as_str()
        .unwrap_or("Damage");
    if where_dur == "Damage" {
        return Some(match item.nbt.as_ref().and_then(|n| n.get("Damage")) {
            Some(NbtTag::Int(i)) => *i,
            _ => 0,
        });
    }
    if where_dur == "metadata" {
        return Some(item.metadata);
    }
    Some(0)
}

pub fn set_durability_used(registry: &Registry, item: &Item, durability: i32) -> Item {
    let where_dur = registry
        .support_feature("whereDurabilityIsSerialized")
        .as_str()
        .unwrap_or("Damage");
    if where_dur == "Damage" {
        return with_nbt_value(item, "Damage", nbt_int(durability));
    }
    if where_dur == "metadata" {
        return Item {
            metadata: durability,
            ..item.clone()
        };
    }
    item.clone()
}

pub fn get_repair_cost(item: &Item) -> i32 {
    match item.nbt.as_ref().and_then(|n| n.get("RepairCost")) {
        Some(NbtTag::Int(i)) => *i,
        _ => 0,
    }
}

pub fn set_repair_cost(item: &Item, cost: i32) -> Item {
    with_nbt_value(item, "RepairCost", nbt_int(cost))
}

// ── Block restrictions ──

fn get_string_list(item: &Item, key: &str) -> Vec<String> {
    match item.nbt.as_ref().and_then(|n| n.get(key)) {
        Some(NbtTag::List(list)) => list
            .items
            .iter()
            .filter_map(|t| t.as_string().map(String::from))
            .collect(),
        _ => vec![],
    }
}

fn set_string_list(item: &Item, key: &str, values: &[String]) -> Item {
    let items = values
        .iter()
        .map(|v| {
            let normalized = if v.contains(':') {
                v.clone()
            } else {
                format!("minecraft:{v}")
            };
            nbt_string(normalized)
        })
        .collect();
    with_nbt_value(item, key, nbt_list(NbtType::String, items))
}

pub fn get_blocks_can_place_on(item: &Item) -> Vec<String> {
    get_string_list(item, "CanPlaceOn")
}
pub fn set_blocks_can_place_on(item: &Item, blocks: &[String]) -> Item {
    set_string_list(item, "CanPlaceOn", blocks)
}
pub fn get_blocks_can_destroy(item: &Item) -> Vec<String> {
    get_string_list(item, "CanDestroy")
}
pub fn set_blocks_can_destroy(item: &Item, blocks: &[String]) -> Item {
    set_string_list(item, "CanDestroy", blocks)
}

// ── Spawn eggs ──

pub fn get_spawn_egg_mob_name(registry: &Registry, item: &Item) -> Option<String> {
    if !item.name.ends_with("_spawn_egg") {
        return None;
    }
    if registry
        .support_feature("spawnEggsHaveSpawnedEntityInName")
        .as_bool()
    {
        return Some(item.name.replace("_spawn_egg", ""));
    }
    if registry
        .support_feature("spawnEggsUseEntityTagInNbt")
        .as_bool()
    {
        if let Some(entity_tag) = get_compound_tag(item.nbt.as_ref(), "EntityTag") {
            if let Some(NbtTag::String(id)) = entity_tag.get("id") {
                return Some(id.replace("minecraft:", ""));
            }
        }
    }
    None
}

// ── Equality ──

pub fn items_equal(a: Option<&Item>, b: Option<&Item>, match_count: bool, match_nbt: bool) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
        (Some(a), Some(b)) => {
            if a.type_id != b.type_id || a.metadata != b.metadata {
                return false;
            }
            if match_count && a.count != b.count {
                return false;
            }
            if match_nbt {
                if a.components.len() != b.components.len() {
                    return false;
                }
                for (ca, cb) in a.components.iter().zip(&b.components) {
                    if ca.ty != cb.ty || ca.hash != cb.hash {
                        return false;
                    }
                }
                if a.removed_components != b.removed_components {
                    return false;
                }
                if a.nbt.is_some() || b.nbt.is_some() {
                    return a.nbt == b.nbt;
                }
            }
            true
        }
    }
}

// ── Network serialization ──

/// Convert raw deserialized components (`{type, data}`) to `ItemComponent`s.
fn hash_components(raw: &[PValue]) -> Vec<ItemComponent> {
    raw.iter()
        .filter_map(|c| {
            let ty = c.get("type")?.as_str()?.to_string();
            let data = c.get("data").cloned().unwrap_or(PValue::Void);
            let hash = hash_component_data(&ty, &data);
            Some(ItemComponent { ty, data, hash })
        })
        .collect()
}

/// Convert an item to the network (HashedSlot) value, 1.21+ path. Empty → `Void`.
pub fn to_notch(registry: &Registry, item: Option<&Item>) -> PValue {
    if registry
        .support_feature("itemSerializationUsesBlockId")
        .as_bool()
    {
        // Legacy blockId format.
        let Some(item) = item else {
            return PValue::compound(vec![
                ("blockId", PValue::num(-1.0)),
                ("itemCount", PValue::num(0.0)),
                ("itemDamage", PValue::num(0.0)),
            ]);
        };
        return PValue::compound(vec![
            ("blockId", PValue::num(item.type_id as f64)),
            ("itemCount", PValue::num(item.count as f64)),
            ("itemDamage", PValue::num(item.metadata as f64)),
        ]);
    }

    let Some(item) = item else {
        return PValue::Void;
    };
    let components = item
        .components
        .iter()
        .map(|c| {
            PValue::compound(vec![
                ("type", PValue::str(c.ty.clone())),
                ("hash", PValue::num(c.hash as f64)),
            ])
        })
        .collect();
    let remove_components = item
        .removed_components
        .iter()
        .map(|t| PValue::compound(vec![("type", PValue::str(t.clone()))]))
        .collect();
    PValue::compound(vec![
        ("itemId", PValue::num(item.type_id as f64)),
        ("itemCount", PValue::num(item.count as f64)),
        ("components", PValue::List(components)),
        ("removeComponents", PValue::List(remove_components)),
    ])
}

/// Convert a network (Slot) value back to an item, or `None` if empty.
pub fn from_notch(registry: &Registry, network: &PValue) -> Option<Item> {
    if network.is_void() {
        return None;
    }
    // 1.21 Slot: { itemCount, itemId, components: [{type, data}], removeComponents: [{type}] }
    if let Some(count) = network.get("itemCount").and_then(PValue::as_i32) {
        if count == 0 {
            return None;
        }
        let id = network.get("itemId").and_then(PValue::as_i32).unwrap_or(0);
        let components = hash_components(
            network
                .get("components")
                .and_then(PValue::as_list)
                .unwrap_or(&[]),
        );
        let removed = network
            .get("removeComponents")
            .and_then(PValue::as_list)
            .unwrap_or(&[])
            .iter()
            .filter_map(|r| r.get("type").and_then(PValue::as_str).map(String::from))
            .collect();
        return Some(create_item(
            registry, id, count, 0, None, components, removed,
        ));
    }
    // Legacy blockId
    if let Some(block_id) = network.get("blockId").and_then(PValue::as_i32) {
        if block_id == -1 {
            return None;
        }
        let count = network
            .get("itemCount")
            .and_then(PValue::as_i32)
            .unwrap_or(0);
        let damage = network
            .get("itemDamage")
            .and_then(PValue::as_i32)
            .unwrap_or(0);
        return Some(create_item(
            registry,
            block_id,
            count,
            damage,
            None,
            vec![],
            vec![],
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{BlockCollisionShapes, ItemDefinition, Registry};

    fn registry() -> Registry {
        Registry::build(
            vec![],
            vec![ItemDefinition {
                id: 5,
                name: "diamond_sword".into(),
                display_name: "Diamond Sword".into(),
                stack_size: 1,
                enchant_categories: None,
                repair_with: None,
                max_durability: Some(1561),
            }],
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
    fn creates_known_and_unknown() {
        let reg = registry();
        let sword = create_item(&reg, 5, 1, 0, None, vec![], vec![]);
        assert_eq!(sword.name, "diamond_sword");
        assert_eq!(sword.max_durability, Some(1561));

        let unknown = create_item(&reg, 99999, 1, 0, None, vec![], vec![]);
        assert_eq!(unknown.name, "unknown");
        assert_eq!(unknown.max_durability, None);
    }

    #[test]
    fn create_by_name_errors_on_unknown() {
        let reg = registry();
        assert!(create_item_by_name(&reg, "not_real", 1, 0, None).is_err());
        assert!(create_item_by_name(&reg, "diamond_sword", 1, 0, None).is_ok());
    }

    #[test]
    fn custom_name_roundtrip() {
        let reg = registry();
        let item = create_item(&reg, 5, 1, 0, None, vec![], vec![]);
        let named = set_custom_name(&item, "Excalibur");
        assert_eq!(get_custom_name(&named).as_deref(), Some("Excalibur"));
        assert_eq!(get_custom_name(&item), None);
    }

    #[test]
    fn custom_lore_roundtrip() {
        let reg = registry();
        let item = create_item(&reg, 5, 1, 0, None, vec![], vec![]);
        let lore = vec!["Line 1".to_string(), "Line 2".to_string()];
        let with_lore = set_custom_lore(&item, &lore);
        assert_eq!(get_custom_lore(&with_lore), Some(lore));
    }

    #[test]
    fn repair_cost_roundtrip() {
        let reg = registry();
        let item = create_item(&reg, 5, 1, 0, None, vec![], vec![]);
        assert_eq!(get_repair_cost(&item), 0);
        let repaired = set_repair_cost(&item, 3);
        assert_eq!(get_repair_cost(&repaired), 3);
    }

    #[test]
    fn enchants_roundtrip_1_21() {
        let reg = registry();
        let sword = create_item(&reg, 5, 1, 0, None, vec![], vec![]);
        let enchanted = set_enchants(
            &reg,
            &sword,
            &[
                Enchant {
                    name: "sharpness".into(),
                    level: 5,
                },
                Enchant {
                    name: "looting".into(),
                    level: 3,
                },
            ],
        );
        let enchants = get_enchants(&reg, &enchanted);
        assert_eq!(enchants.len(), 2);
        assert_eq!(
            enchants[0],
            Enchant {
                name: "sharpness".into(),
                level: 5
            }
        );
        assert_eq!(
            enchants[1],
            Enchant {
                name: "looting".into(),
                level: 3
            }
        );
    }

    #[test]
    fn durability() {
        let reg = registry();
        let sword = create_item(&reg, 5, 1, 0, None, vec![], vec![]);
        assert_eq!(get_durability_used(&reg, &sword), Some(0));
        let damaged = set_durability_used(&reg, &sword, 100);
        assert_eq!(get_durability_used(&reg, &damaged), Some(100));
    }

    #[test]
    fn equality_and_notch_empty() {
        let reg = registry();
        let a = create_item(&reg, 5, 1, 0, None, vec![], vec![]);
        let b = create_item(&reg, 5, 1, 0, None, vec![], vec![]);
        assert!(items_equal(Some(&a), Some(&b), true, true));
        assert!(items_equal(None, None, true, true));
        assert!(!items_equal(Some(&a), None, true, true));

        // empty slot roundtrips through Void
        assert!(to_notch(&reg, None).is_void());
        assert!(from_notch(&reg, &PValue::Void).is_none());
    }

    #[test]
    fn notch_roundtrip_present() {
        let reg = registry();
        let item = create_item(&reg, 5, 2, 0, None, vec![], vec![]);
        let notch = to_notch(&reg, Some(&item));
        // to_notch produces HashedSlot (itemId/itemCount); simulate a Slot read for from_notch
        let slot = PValue::compound(vec![
            ("itemCount", PValue::num(2.0)),
            ("itemId", PValue::num(5.0)),
            ("components", PValue::List(vec![])),
            ("removeComponents", PValue::List(vec![])),
        ]);
        assert_eq!(notch.get("itemId").unwrap().as_i32(), Some(5));
        let back = from_notch(&reg, &slot).unwrap();
        assert_eq!(back.type_id, 5);
        assert_eq!(back.count, 2);
    }
}
