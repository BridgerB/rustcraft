//! Minecraft entities — players, mobs, objects. Port of typecraft's `entity`
//! module. Vehicle/passenger links use entity ids (the world keeps entities in
//! a map) rather than owned references, avoiding ownership cycles.

use std::collections::HashMap;

use crate::item::Item;
use crate::protocol::PValue;
use crate::registry::Registry;
use crate::vec3::{Vec3, ZERO};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    Player,
    Mob,
    Object,
    Global,
    Orb,
    Projectile,
    Hostile,
    Other,
}

impl EntityType {
    pub fn from_str(s: &str) -> EntityType {
        match s {
            "player" => EntityType::Player,
            "mob" => EntityType::Mob,
            "object" => EntityType::Object,
            "global" => EntityType::Global,
            "orb" => EntityType::Orb,
            "projectile" => EntityType::Projectile,
            "hostile" => EntityType::Hostile,
            _ => EntityType::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effect {
    pub id: i32,
    pub amplifier: i32,
    pub duration: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttributeModifier {
    pub uuid: String,
    pub amount: f64,
    pub operation: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityAttribute {
    pub value: f64,
    pub modifiers: Vec<AttributeModifier>,
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub id: i32,
    pub ty: EntityType,
    pub uuid: Option<String>,
    pub username: Option<String>,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub entity_type: Option<i32>,
    pub kind: Option<String>,
    pub position: Vec3,
    pub velocity: Vec3,
    pub yaw: f64,
    pub pitch: f64,
    pub on_ground: bool,
    pub height: f64,
    pub width: f64,
    pub equipment: Vec<Option<Item>>,
    pub metadata: Vec<PValue>,
    pub effects: HashMap<i32, Effect>,
    pub attributes: HashMap<String, EntityAttribute>,
    pub vehicle: Option<i32>,
    pub passengers: Vec<i32>,
    pub health: f64,
    pub food: f64,
    pub food_saturation: f64,
    pub is_in_water: bool,
    pub elytra_flying: bool,
    pub is_valid: bool,
    pub count: Option<i32>,
    pub fixed_x: i64,
    pub fixed_y: i64,
    pub fixed_z: i64,
}

impl Entity {
    pub fn new(id: i32) -> Entity {
        Entity {
            id,
            ty: EntityType::Other,
            uuid: None,
            username: None,
            name: None,
            display_name: None,
            entity_type: None,
            kind: None,
            position: ZERO,
            velocity: ZERO,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            height: 0.0,
            width: 0.0,
            equipment: vec![None; 6],
            metadata: Vec::new(),
            effects: HashMap::new(),
            attributes: HashMap::new(),
            vehicle: None,
            passengers: Vec::new(),
            health: 20.0,
            food: 20.0,
            food_saturation: 5.0,
            is_in_water: false,
            elytra_flying: false,
            is_valid: true,
            count: None,
            fixed_x: 0,
            fixed_y: 0,
            fixed_z: 0,
        }
    }

    /// Initialize fields from the registry entity definition.
    pub fn init(&mut self, registry: &Registry, entity_type_id: i32) {
        let Some(def) = registry.entities_by_id.get(&entity_type_id) else {
            return;
        };
        self.entity_type = Some(entity_type_id);
        self.name = Some(def.name.clone());
        self.display_name = Some(def.display_name.clone());
        self.height = def.height;
        self.width = def.width;
        self.kind = Some(def.category.clone());
        self.ty = EntityType::from_str(&def.ty);
    }

    // ── Equipment ──

    pub fn set_equipment(&mut self, slot: usize, item: Option<Item>) {
        if slot < self.equipment.len() {
            self.equipment[slot] = item;
        }
    }

    pub fn held_item(&self) -> Option<&Item> {
        self.equipment.first().and_then(|i| i.as_ref())
    }

    pub fn offhand_item(&self) -> Option<&Item> {
        self.equipment.get(1).and_then(|i| i.as_ref())
    }

    /// Armor slots (boots, leggings, chestplate, helmet).
    pub fn armor(&self) -> &[Option<Item>] {
        &self.equipment[2..]
    }

    // ── Effects ──

    pub fn add_effect(&mut self, effect: Effect) {
        self.effects.insert(effect.id, effect);
    }

    pub fn remove_effect(&mut self, effect_id: i32) {
        self.effects.remove(&effect_id);
    }

    pub fn clear_effects(&mut self) {
        self.effects.clear();
    }

    // ── Vehicle / passengers ──

    pub fn set_vehicle(&mut self, vehicle: Option<i32>) {
        self.vehicle = vehicle;
    }

    pub fn add_passenger(&mut self, passenger: i32) {
        self.passengers.push(passenger);
    }

    pub fn remove_passenger(&mut self, passenger_id: i32) {
        self.passengers.retain(|&p| p != passenger_id);
    }

    // ── Validity ──

    pub fn valid(&self) -> bool {
        self.is_valid
    }

    pub fn invalidate(&mut self) {
        self.is_valid = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let e = Entity::new(1);
        assert_eq!(e.id, 1);
        assert_eq!(e.health, 20.0);
        assert_eq!(e.equipment.len(), 6);
        assert!(e.valid());
    }

    #[test]
    fn equipment_slots() {
        let mut e = Entity::new(1);
        assert!(e.held_item().is_none());
        assert_eq!(e.armor().len(), 4);
        e.set_equipment(0, None);
        assert!(e.held_item().is_none());
    }

    #[test]
    fn effects() {
        let mut e = Entity::new(1);
        e.add_effect(Effect {
            id: 1,
            amplifier: 0,
            duration: 200,
        });
        assert_eq!(e.effects.len(), 1);
        e.remove_effect(1);
        assert!(e.effects.is_empty());
    }

    #[test]
    fn passengers_and_validity() {
        let mut e = Entity::new(1);
        e.add_passenger(2);
        e.add_passenger(3);
        e.remove_passenger(2);
        assert_eq!(e.passengers, vec![3]);
        e.invalidate();
        assert!(!e.valid());
    }
}
