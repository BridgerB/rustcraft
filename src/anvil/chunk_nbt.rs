//! Convert between `ChunkColumn` and anvil-format NBT.

use crate::chunk::{needed_bits, BitArray, ChunkColumn, ChunkColumnOptions, PaletteContainer};
use crate::nbt::{
    compound, nbt_byte, nbt_byte_array, nbt_int, nbt_list, nbt_long, nbt_long_array, nbt_string,
    NbtCompound, NbtList, NbtRoot, NbtTag, NbtType,
};
use crate::registry::Registry;

fn tag_i32(t: &NbtTag) -> i32 {
    match t {
        NbtTag::Byte(b) => *b as i32,
        NbtTag::Short(s) => *s as i32,
        NbtTag::Int(i) => *i,
        NbtTag::Long(l) => *l as i32,
        _ => 0,
    }
}

fn long_array(t: Option<&NbtTag>) -> Option<&[i64]> {
    match t {
        Some(NbtTag::LongArray(v)) => Some(v),
        _ => None,
    }
}

fn empty_bit_array() -> BitArray {
    BitArray::from_data(Vec::new(), 1, 0)
}

/// Convert an anvil-format NBT chunk to a `ChunkColumn`.
pub fn nbt_to_chunk_column(nbt_root: &NbtRoot, registry: &Registry) -> ChunkColumn {
    let max_block_state_id = registry
        .blocks_array
        .iter()
        .map(|b| b.max_state_id)
        .max()
        .unwrap_or(0);
    let mut col = ChunkColumn::new(ChunkColumnOptions {
        min_y: Some(-64),
        world_height: Some(384),
        max_bits_per_block: needed_bits(max_block_state_id),
        max_bits_per_biome: needed_bits(registry.biomes_array.len() as u32),
    });

    // Block entities (preserve raw NBT).
    if let Some(NbtTag::List(list)) = nbt_root.value.get("block_entities") {
        if list.ty == NbtType::Compound {
            for entry in &list.items {
                if let NbtTag::Compound(c) = entry {
                    let x = c.get("x").map(tag_i32);
                    let y = c.get("y").map(tag_i32);
                    let z = c.get("z").map(tag_i32);
                    if let (Some(x), Some(y), Some(z)) = (x, y, z) {
                        col.block_entities.insert((x & 0xf, y, z & 0xf), c.clone());
                    }
                }
            }
        }
    }

    let Some(NbtTag::List(sections)) = nbt_root.value.get("sections") else {
        return col;
    };

    for section in &sections.items {
        let NbtTag::Compound(section) = section else {
            continue;
        };
        let Some(section_y) = section.get("Y").map(tag_i32) else {
            continue;
        };
        let (Some(NbtTag::Compound(block_states)), Some(NbtTag::Compound(biomes))) =
            (section.get("block_states"), section.get("biomes"))
        else {
            continue;
        };
        let (Some(NbtTag::List(block_palette)), Some(NbtTag::List(biome_palette))) =
            (block_states.get("palette"), biomes.get("palette"))
        else {
            continue;
        };

        let mut bits_per_block = log2_ceil(block_palette.items.len());
        if (1..=3).contains(&bits_per_block) {
            bits_per_block = 4;
        }
        let bits_per_biome = log2_ceil(biome_palette.items.len());

        let block_data = match long_array(block_states.get("data")) {
            Some(longs) => BitArray::from_long_array(longs, bits_per_block.max(1)),
            None => empty_bit_array(),
        };
        let biome_data = match long_array(biomes.get("data")) {
            Some(longs) => BitArray::from_long_array(longs, bits_per_biome.max(1)),
            None => empty_bit_array(),
        };

        let mapped_block_palette: Vec<u32> = block_palette
            .items
            .iter()
            .map(|entry| {
                let NbtTag::Compound(c) = entry else {
                    return 0;
                };
                let name = c
                    .get("Name")
                    .and_then(NbtTag::as_string)
                    .unwrap_or("air")
                    .replace("minecraft:", "");
                let props = match c.get("Properties") {
                    Some(NbtTag::Compound(p)) => compound_string_map(p),
                    _ => Vec::new(),
                };
                block_name_to_state_id(registry, &name, &props)
            })
            .collect();

        let mapped_biome_palette: Vec<u32> = biome_palette
            .items
            .iter()
            .map(|entry| {
                let name = entry
                    .as_string()
                    .unwrap_or("plains")
                    .replace("minecraft:", "");
                registry
                    .biomes_by_name
                    .get(&name)
                    .map(|b| b.id as u32)
                    .unwrap_or(0)
            })
            .collect();

        let block_light = byte_array_bytes(section.get("BlockLight"));
        let sky_light = byte_array_bytes(section.get("SkyLight"));

        col.load_section_from_anvil(
            section_y,
            mapped_block_palette,
            block_data,
            mapped_biome_palette,
            biome_data,
            block_light.as_deref(),
            sky_light.as_deref(),
        );
    }

    col
}

/// Convert a `ChunkColumn` to anvil-format NBT.
pub fn chunk_column_to_nbt(
    col: &ChunkColumn,
    chunk_x: i32,
    chunk_z: i32,
    registry: &Registry,
    data_version: i32,
) -> NbtRoot {
    let mut section_tags: Vec<NbtTag> = Vec::new();
    let min_section_y = col.min_y >> 4;
    let max_section_y = min_section_y + col.num_sections as i32;

    for y in min_section_y..max_section_y {
        let idx = (y - min_section_y) as usize;
        let section = &col.sections[idx];
        let biome_section = &col.biomes[idx];

        let (block_entries, block_bits, block_bitarr) = block_palette_nbt(&section.data, registry);
        let (biome_names, biome_bits, biome_bitarr) =
            biome_palette_nbt(&biome_section.data, registry);

        let mut block_states = vec![(
            "palette".to_string(),
            nbt_list(NbtType::Compound, block_entries),
        )];
        if block_bits > 0 {
            if let Some(arr) = block_bitarr {
                block_states.push(("data".to_string(), nbt_long_array(arr.to_long_array())));
            }
        }

        let mut biomes = vec![(
            "palette".to_string(),
            nbt_list(
                NbtType::String,
                biome_names.into_iter().map(nbt_string).collect(),
            ),
        )];
        if biome_bits > 0 {
            if let Some(arr) = biome_bitarr {
                biomes.push(("data".to_string(), nbt_long_array(arr.to_long_array())));
            }
        }

        let mut section_entries: Vec<(String, NbtTag)> = vec![
            ("Y".into(), nbt_byte(y as i8)),
            (
                "block_states".into(),
                NbtTag::Compound(block_states.into_iter().collect()),
            ),
            (
                "biomes".into(),
                NbtTag::Compound(biomes.into_iter().collect()),
            ),
        ];

        if let Some(light) = &col.block_light_sections[idx + 1] {
            section_entries.push(("BlockLight".into(), nbt_byte_array(light_bytes(light))));
        }
        if let Some(light) = &col.sky_light_sections[idx + 1] {
            section_entries.push(("SkyLight".into(), nbt_byte_array(light_bytes(light))));
        }

        section_tags.push(NbtTag::Compound(section_entries.into_iter().collect()));
    }

    NbtRoot {
        name: String::new(),
        value: compound(vec![
            ("DataVersion", nbt_int(data_version)),
            ("Status", nbt_string("full")),
            ("xPos", nbt_int(chunk_x)),
            ("yPos", nbt_int(col.min_y >> 4)),
            ("zPos", nbt_int(chunk_z)),
            ("sections", nbt_list(NbtType::Compound, section_tags)),
            ("block_entities", serialize_block_entities(col)),
            ("LastUpdate", nbt_long(0)),
            ("InhabitedTime", nbt_long(0)),
            ("structures", NbtTag::Compound(NbtCompound::new())),
            ("Heightmaps", NbtTag::Compound(NbtCompound::new())),
            ("isLightOn", nbt_int(0)),
            ("block_ticks", NbtTag::List(NbtList::empty())),
            ("PostProcessing", NbtTag::List(NbtList::empty())),
            ("fluid_ticks", NbtTag::List(NbtList::empty())),
        ]),
    }
}

// ── Helpers ──

fn log2_ceil(n: usize) -> u32 {
    if n <= 1 {
        0
    } else {
        usize::BITS - (n - 1).leading_zeros()
    }
}

fn compound_string_map(c: &NbtCompound) -> Vec<(String, String)> {
    c.iter()
        .filter_map(|(k, v)| v.as_string().map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn byte_array_bytes(t: Option<&NbtTag>) -> Option<Vec<u8>> {
    match t {
        Some(NbtTag::ByteArray(v)) => Some(v.iter().map(|&b| b as u8).collect()),
        _ => None,
    }
}

fn light_bytes(arr: &BitArray) -> Vec<i8> {
    arr.data
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .map(|b| b as i8)
        .collect()
}

fn block_name_to_state_id(registry: &Registry, name: &str, properties: &[(String, String)]) -> u32 {
    let Some(block) = registry.blocks_by_name.get(name) else {
        return 0;
    };
    if properties.is_empty() || block.states.is_empty() {
        return block.default_state;
    }
    let mut offset = 0u32;
    let mut multiplier = 1u32;
    for prop in block.states.iter().rev() {
        if let Some((_, value)) = properties.iter().find(|(k, _)| *k == prop.name) {
            if let Some(values) = &prop.values {
                if let Some(idx) = values.iter().position(|v| v == value) {
                    offset += idx as u32 * multiplier;
                }
            }
        }
        multiplier *= prop.num_values;
    }
    block.min_state_id + offset
}

fn state_id_to_block_nbt(registry: &Registry, state_id: u32) -> NbtTag {
    let Some(block) = registry.blocks_by_state_id.get(&state_id) else {
        return NbtTag::Compound(compound(vec![("Name", nbt_string("minecraft:air"))]));
    };
    let mut entries = vec![(
        "Name".to_string(),
        nbt_string(format!("minecraft:{}", block.name)),
    )];

    if !block.states.is_empty() && state_id != block.default_state {
        let mut props: Vec<(String, NbtTag)> = Vec::new();
        let mut remaining = state_id - block.min_state_id;
        let mut multiplier: u32 = block.states.iter().map(|s| s.num_values).product();
        for prop in &block.states {
            multiplier /= prop.num_values;
            let idx = (remaining / multiplier) as usize;
            remaining %= multiplier;
            if let Some(values) = &prop.values {
                let v = values.get(idx).cloned().unwrap_or_else(|| idx.to_string());
                props.push((prop.name.clone(), nbt_string(v)));
            }
        }
        if !props.is_empty() {
            entries.push((
                "Properties".into(),
                NbtTag::Compound(props.into_iter().collect()),
            ));
        }
    }

    NbtTag::Compound(entries.into_iter().collect())
}

/// Returns (palette nbt entries, bits_per_value, optional bit array for `data`).
fn block_palette_nbt<'a>(
    container: &'a PaletteContainer,
    registry: &Registry,
) -> (Vec<NbtTag>, u32, Option<&'a BitArray>) {
    match container {
        PaletteContainer::Single(c) => (vec![state_id_to_block_nbt(registry, c.value)], 0, None),
        PaletteContainer::Indirect(c) => {
            let entries = c
                .palette
                .iter()
                .map(|&id| state_id_to_block_nbt(registry, id))
                .collect();
            (entries, log2_ceil(c.palette.len()), Some(&c.data))
        }
        PaletteContainer::Direct(c) => {
            let mut seen = Vec::new();
            let mut entries = Vec::new();
            for i in 0..c.data.capacity {
                let id = c.data.get(i);
                if !seen.contains(&id) {
                    seen.push(id);
                    entries.push(state_id_to_block_nbt(registry, id));
                }
            }
            (entries, log2_ceil(seen.len()), Some(&c.data))
        }
    }
}

fn biome_palette_nbt<'a>(
    container: &'a PaletteContainer,
    registry: &Registry,
) -> (Vec<String>, u32, Option<&'a BitArray>) {
    let biome_name = |id: u32| {
        registry
            .biomes_by_id
            .get(&(id as i32))
            .map(|b| format!("minecraft:{}", b.name))
            .unwrap_or_else(|| "minecraft:plains".to_string())
    };
    match container {
        PaletteContainer::Single(c) => (vec![biome_name(c.value)], 0, None),
        PaletteContainer::Indirect(c) => {
            let names = c.palette.iter().map(|&id| biome_name(id)).collect();
            (names, log2_ceil(c.palette.len()), Some(&c.data))
        }
        PaletteContainer::Direct(c) => {
            let mut seen = Vec::new();
            let mut names = Vec::new();
            for i in 0..c.data.capacity {
                let id = c.data.get(i);
                if !seen.contains(&id) {
                    seen.push(id);
                    names.push(biome_name(id));
                }
            }
            (names, log2_ceil(seen.len()), Some(&c.data))
        }
    }
}

fn serialize_block_entities(col: &ChunkColumn) -> NbtTag {
    if col.block_entities.is_empty() {
        return NbtTag::List(NbtList::empty());
    }
    let compounds = col
        .block_entities
        .values()
        .map(|c| NbtTag::Compound(c.clone()))
        .collect();
    nbt_list(NbtType::Compound, compounds)
}
