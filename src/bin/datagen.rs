//! rustcraft's data generator.
//!
//! Downloads the Minecraft server JAR, runs the vanilla data generator
//! (`--reports --server`), and transforms the reports into the JSON format the
//! `registry` module loads: blocks/items/entities/effects/attributes,
//! synthetic collision shapes, plus recipes/tags/biomes copied from the
//! generated data.
//!
//! Usage: `cargo run --bin datagen [-- <output-dir>]` (default `data/`).
//! The server JAR + reports are cached under `datagen/work/` (idempotent).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Map, Value};

const SERVER_JAR_URL: &str =
    "https://piston-data.mojang.com/v1/objects/97ccd4c0ed3f81bbb7bfacddd1090b0c56f9bc51/server.jar";
const VERSION: &str = "26.1.2";

/// Block `definition.type`s that have no full-cube collision (passable).
const NON_SOLID_TYPES: &[&str] = &[
    "air",
    "flower",
    "flower_pot",
    "banner",
    "wall_banner",
    "button",
    "pressure_plate",
    "weighted_pressure_plate",
    "sapling",
    "bamboo_sapling",
    "coral_plant",
    "coral",
    "coral_fan",
    "coral_wall_fan",
    "base_coral_plant",
    "base_coral_fan",
    "base_coral_wall_fan",
    "tall_flower",
    "torch",
    "wall_torch",
    "double_plant",
    "powered_rail",
    "detector_rail",
    "rail",
    "mushroom",
    "tall_grass",
    "grass",
    "short_dry_grass",
    "tall_dry_grass",
    "liquid",
    "flower_bed",
    "bush",
    "cactus_flower",
    "cave_vines",
    "cave_vines_plant",
    "web",
    "fire",
    "soul_fire",
    "firefly_bush",
    "glow_lichen",
    "kelp",
    "kelp_plant",
    "lever",
    "carpet",
    "wool_carpet",
    "mossy_carpet",
    "pitcher_crop",
    "redstone_torch",
    "redstone_wall_torch",
    "redstone_wire",
    "seagrass",
    "tall_seagrass",
    "sugar_cane",
    "sweet_berry_bush",
    "torchflower_crop",
    "tripwire",
    "trip_wire_hook",
    "twisting_vines",
    "twisting_vines_plant",
    "vine",
    "weeping_vines",
    "weeping_vines_plant",
    "crop",
    "standing_sign",
    "wall_sign",
    "ceiling_hanging_sign",
    "wall_hanging_sign",
];

fn find_java() -> String {
    if let Ok(j) = std::env::var("JAVA") {
        return j;
    }
    for candidate in [
        "/opt/homebrew/opt/openjdk@26/bin/java",
        "/opt/homebrew/opt/openjdk@25/bin/java",
        "/opt/homebrew/opt/openjdk/bin/java",
        "/usr/bin/java",
    ] {
        if Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    "java".to_string()
}

fn short_name(resource: &str) -> &str {
    resource.strip_prefix("minecraft:").unwrap_or(resource)
}

fn display_name(name: &str) -> String {
    name.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn load_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn registry_entries<'a>(registries: &'a Value, key: &str) -> &'a Map<String, Value> {
    registries[key]["entries"]
        .as_object()
        .unwrap_or_else(|| panic!("missing registry {key}"))
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<usize> {
    fs::create_dir_all(dst)?;
    let mut count = 0;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            count += copy_dir(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
            count += 1;
        }
    }
    Ok(count)
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));
    let work = PathBuf::from("datagen/work");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&out).unwrap();

    println!("=== rustcraft datagen — Minecraft {VERSION} ===");

    // 1. Download the server JAR.
    let jar = work.join("server.jar");
    if !jar.exists() {
        println!("Downloading server.jar…");
        let status = Command::new("curl")
            .args(["-sL", "-o", jar.to_str().unwrap(), SERVER_JAR_URL])
            .status()
            .expect("run curl");
        assert!(status.success(), "download failed");
    }

    // 2. Run the vanilla data generator.
    let report_dir = work.join("output");
    if !report_dir.join("reports/blocks.json").exists() {
        let java = find_java();
        println!("Running data generator with {java}…");
        let status = Command::new(&java)
            .current_dir(&work)
            .args([
                "-DbundlerMainClass=net.minecraft.data.Main",
                "-jar",
                "server.jar",
                "--reports",
                "--server",
                "--output",
                "output",
            ])
            .status()
            .expect("run java");
        assert!(status.success(), "data generator failed");
    }

    let reports = report_dir.join("reports");
    let data = report_dir.join("data/minecraft");

    let registries = load_json(&reports.join("registries.json"));
    let blocks_report = load_json(&reports.join("blocks.json"));

    // 3. Transform blocks.
    println!("Transforming blocks…");
    let block_ids = registry_entries(&registries, "minecraft:block");
    let mut blocks = Vec::new();
    let mut collision_blocks = Map::new();
    let blocks_obj = blocks_report.as_object().unwrap();
    for (name_res, info) in blocks_obj {
        let name = short_name(name_res);
        let id = block_ids
            .get(name_res)
            .and_then(|v| v["protocol_id"].as_i64())
            .unwrap_or(-1);
        let def_type = short_name(info["definition"]["type"].as_str().unwrap_or(""));
        let states = info["states"].as_array().cloned().unwrap_or_default();

        let mut min_state = i64::MAX;
        let mut max_state = i64::MIN;
        let mut default_state = 0i64;
        // Property name order from the first state; value order by first appearance.
        let mut prop_order: Vec<String> = Vec::new();
        let mut prop_values: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (i, st) in states.iter().enumerate() {
            let sid = st["id"].as_i64().unwrap_or(0);
            min_state = min_state.min(sid);
            max_state = max_state.max(sid);
            if st["default"].as_bool().unwrap_or(false) {
                default_state = sid;
            }
            if let Some(props) = st["properties"].as_object() {
                if i == 0 {
                    prop_order = props.keys().cloned().collect();
                }
                for (k, v) in props {
                    let val = v.as_str().unwrap_or("").to_string();
                    let entry = prop_values.entry(k.clone()).or_default();
                    if !entry.contains(&val) {
                        entry.push(val);
                    }
                }
            }
        }
        if min_state == i64::MAX {
            min_state = 0;
            max_state = 0;
        }
        if states.len() == 1 {
            default_state = min_state;
        }

        let state_props: Vec<Value> = prop_order
            .iter()
            .map(|name| {
                let values = prop_values.get(name).cloned().unwrap_or_default();
                let ty = if values == ["true", "false"] {
                    "bool"
                } else if values.iter().all(|v| v.parse::<i64>().is_ok()) {
                    "int"
                } else {
                    "enum"
                };
                json!({ "name": name, "type": ty, "num_values": values.len(), "values": values })
            })
            .collect();

        // A non-solid TYPE (e.g. the grass plant) marks a block passable — BUT
        // full "_block" blocks (notably grass_block, whose definition type is also
        // "grass") are solid cubes. Without this, the bot falls through grass.
        let solid = !NON_SOLID_TYPES.contains(&def_type) || name.ends_with("_block");
        let bounding_box = if solid { "block" } else { "empty" };
        collision_blocks.insert(name.to_string(), json!(if solid { 1 } else { 0 }));

        blocks.push(json!({
            "id": id,
            "name": name,
            "displayName": display_name(name),
            "hardness": if solid { 1.0 } else { 0.0 },
            "resistance": if solid { 1.0 } else { 0.0 },
            "stackSize": 64,
            "diggable": true,
            "boundingBox": bounding_box,
            "transparent": !solid,
            "emitLight": 0,
            "filterLight": if solid { 15 } else { 0 },
            "defaultState": default_state,
            "minStateId": min_state,
            "maxStateId": max_state,
            "states": state_props,
            "drops": []
        }));
    }
    blocks.sort_by_key(|b| b["id"].as_i64().unwrap_or(0));
    write(&out.join("blocks.json"), &Value::Array(blocks));

    // Synthetic collision shapes: 0 = empty, 1 = full cube.
    let collision = json!({
        "blocks": collision_blocks,
        "shapes": { "0": [], "1": [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]] }
    });
    write(&out.join("blockCollisionShapes.json"), &collision);

    // 4. Items, entities, effects, attributes from registries.
    println!("Transforming items / entities / effects / attributes…");
    write(
        &out.join("items.json"),
        &simple_registry(&registries, "minecraft:item", true),
    );
    write(&out.join("entities.json"), &entities(&registries));
    write(&out.join("effects.json"), &effects(&registries));
    write(&out.join("attributes.json"), &attributes(&registries));

    // 5. Copy recipes, tags, biomes, packets.
    if data.join("recipe").is_dir() {
        let n = copy_dir(&data.join("recipe"), &out.join("recipes-raw")).unwrap_or(0);
        println!("Copied {n} recipes");
    }
    if data.join("tags/item").is_dir() {
        let n = copy_dir(&data.join("tags/item"), &out.join("tags/item")).unwrap_or(0);
        println!("Copied {n} item tags");
    }
    if data.join("worldgen/biome").is_dir() {
        let n = copy_dir(&data.join("worldgen/biome"), &out.join("biomes-raw")).unwrap_or(0);
        println!("Copied {n} biomes");
    }
    if reports.join("packets.json").exists() {
        fs::copy(reports.join("packets.json"), out.join("packets-raw.json")).ok();
    }

    println!("\n✓ Wrote registry data to {}/", out.display());
    println!("  blocks: {}", count_array(&out.join("blocks.json")));
    println!("  items:  {}", count_array(&out.join("items.json")));
}

fn simple_registry(registries: &Value, key: &str, with_stack: bool) -> Value {
    let entries = registry_entries(registries, key);
    let mut arr: Vec<Value> = entries
        .iter()
        .map(|(res, info)| {
            let name = short_name(res);
            let id = info["protocol_id"].as_i64().unwrap_or(-1);
            if with_stack {
                json!({ "id": id, "name": name, "displayName": display_name(name), "stackSize": 64 })
            } else {
                json!({ "id": id, "name": name, "displayName": display_name(name) })
            }
        })
        .collect();
    arr.sort_by_key(|v| v["id"].as_i64().unwrap_or(0));
    Value::Array(arr)
}

fn entities(registries: &Value) -> Value {
    let entries = registry_entries(registries, "minecraft:entity_type");
    let mut arr: Vec<Value> = entries
        .iter()
        .map(|(res, info)| {
            let name = short_name(res);
            json!({
                "id": info["protocol_id"].as_i64().unwrap_or(-1),
                "name": name,
                "displayName": display_name(name),
                "width": 0.6,
                "height": 1.8,
                "type": "mob",
                "category": "Generic"
            })
        })
        .collect();
    arr.sort_by_key(|v| v["id"].as_i64().unwrap_or(0));
    Value::Array(arr)
}

fn effects(registries: &Value) -> Value {
    let entries = registry_entries(registries, "minecraft:mob_effect");
    let mut arr: Vec<Value> = entries
        .iter()
        .map(|(res, info)| {
            let name = short_name(res);
            json!({
                "id": info["protocol_id"].as_i64().unwrap_or(-1),
                "name": name,
                "displayName": display_name(name),
                "type": "good"
            })
        })
        .collect();
    arr.sort_by_key(|v| v["id"].as_i64().unwrap_or(0));
    Value::Array(arr)
}

fn attributes(registries: &Value) -> Value {
    // The registry key changed across versions; accept either.
    let key = if registries.get("minecraft:attribute").is_some() {
        "minecraft:attribute"
    } else {
        "minecraft:attribute_type"
    };
    let entries = registry_entries(registries, key);
    let names: BTreeSet<&str> = entries.keys().map(|s| s.as_str()).collect();
    let arr: Vec<Value> = names
        .iter()
        .map(|res| {
            let name = short_name(res);
            // camelCase the resource for `name`, keep `resource` as the full path.
            json!({
                "resource": format!("minecraft:{name}"),
                "name": camel_case(name),
                "min": 0.0,
                "max": 1024.0,
                "default": 0.0
            })
        })
        .collect();
    Value::Array(arr)
}

fn camel_case(s: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in s.chars() {
        if c == '_' || c == '.' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn write(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(path, serde_json::to_string(value).unwrap()).unwrap();
}

fn count_array(path: &Path) -> usize {
    load_json(path).as_array().map(|a| a.len()).unwrap_or(0)
}
