//! Anvil world storage — region files, `level.dat`, and chunk↔NBT conversion.
//! Port of typecraft's `anvil` module (synchronous `std::fs`).

mod chunk_nbt;
mod level_dat;
mod region;

pub use chunk_nbt::{chunk_column_to_nbt, nbt_to_chunk_column};
pub use level_dat::{read_level_dat, write_level_dat, LevelData};
pub use region::RegionFile;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::chunk::ChunkColumn;
use crate::registry::Registry;

/// An open anvil world directory, caching region file handles.
pub struct AnvilWorld<'a> {
    pub path: PathBuf,
    pub registry: &'a Registry,
    regions: HashMap<PathBuf, RegionFile>,
}

impl<'a> AnvilWorld<'a> {
    pub fn open(path: impl Into<PathBuf>, registry: &'a Registry) -> AnvilWorld<'a> {
        AnvilWorld {
            path: path.into(),
            registry,
            regions: HashMap::new(),
        }
    }

    fn region_path(&self, chunk_x: i32, chunk_z: i32) -> PathBuf {
        let region_x = chunk_x >> 5;
        let region_z = chunk_z >> 5;
        self.path
            .join("region")
            .join(format!("r.{region_x}.{region_z}.mca"))
    }

    fn region(&mut self, chunk_x: i32, chunk_z: i32) -> std::io::Result<&mut RegionFile> {
        let name = self.region_path(chunk_x, chunk_z);
        if !self.regions.contains_key(&name) {
            let region = RegionFile::open(&name)?;
            self.regions.insert(name.clone(), region);
        }
        Ok(self.regions.get_mut(&name).unwrap())
    }

    /// Load a chunk at (x, z) chunk coordinates. `None` if not generated.
    pub fn load_chunk(
        &mut self,
        chunk_x: i32,
        chunk_z: i32,
    ) -> std::io::Result<Option<ChunkColumn>> {
        let local_x = (chunk_x.rem_euclid(32)) as usize;
        let local_z = (chunk_z.rem_euclid(32)) as usize;
        let registry = self.registry;
        let region = self.region(chunk_x, chunk_z)?;
        if !region.has_chunk(local_x, local_z) {
            return Ok(None);
        }
        match region.read_chunk(local_x, local_z)? {
            Some(nbt) => Ok(Some(nbt_to_chunk_column(&nbt, registry))),
            None => Ok(None),
        }
    }

    /// Save a chunk at (x, z) chunk coordinates.
    pub fn save_chunk(
        &mut self,
        chunk_x: i32,
        chunk_z: i32,
        column: &ChunkColumn,
    ) -> std::io::Result<()> {
        let local_x = (chunk_x.rem_euclid(32)) as usize;
        let local_z = (chunk_z.rem_euclid(32)) as usize;
        let data_version = self.registry.version.data_version;
        let nbt = chunk_column_to_nbt(column, chunk_x, chunk_z, self.registry, data_version);
        let region = self.region(chunk_x, chunk_z)?;
        region.write_chunk(local_x, local_z, &nbt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkColumnOptions;
    use crate::registry::{BiomeDefinition, BlockCollisionShapes, BlockDefinition, Registry};

    fn block(name: &str, id: i32, state: u32) -> BlockDefinition {
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
            default_state: state,
            min_state_id: state,
            max_state_id: state,
            states: vec![],
            drops: vec![],
            harvest_tools: None,
        }
    }

    fn registry() -> Registry {
        Registry::build(
            vec![
                block("air", 0, 0),
                block("stone", 1, 1),
                block("dirt", 2, 10),
            ],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![BiomeDefinition {
                id: 0,
                name: "plains".into(),
                display_name: "plains".into(),
                category: "none".into(),
                temperature: 0.8,
                dimension: "overworld".into(),
                color: 0,
                rainfall: None,
            }],
            BlockCollisionShapes::default(),
            std::collections::HashMap::new(),
            "26.1.2",
        )
    }

    fn make_column() -> ChunkColumn {
        ChunkColumn::new(ChunkColumnOptions {
            min_y: None,
            world_height: None,
            max_bits_per_block: 15,
            max_bits_per_biome: 7,
        })
    }

    #[test]
    fn chunk_nbt_roundtrips_blocks() {
        let reg = registry();
        let mut col = make_column();
        col.set_block_state_id(0, 0, 0, 1);
        col.set_block_state_id(3, 5, 7, 10);
        col.set_block_state_id(15, -64, 15, 1);

        let nbt = chunk_column_to_nbt(&col, 0, 0, &reg, 4384);
        let restored = nbt_to_chunk_column(&nbt, &reg);

        assert_eq!(restored.get_block_state_id(0, 0, 0), 1);
        assert_eq!(restored.get_block_state_id(3, 5, 7), 10);
        assert_eq!(restored.get_block_state_id(15, -64, 15), 1);
        assert_eq!(restored.get_block_state_id(1, 1, 1), 0);
    }

    #[test]
    fn region_file_roundtrips_chunk() {
        let reg = registry();
        let dir = std::env::temp_dir().join(format!("rustcraft-world-{}", std::process::id()));
        let mut col = make_column();
        col.set_block_state_id(2, 3, 4, 10);

        {
            let mut world = AnvilWorld::open(&dir, &reg);
            world.save_chunk(0, 0, &col).unwrap();
        }
        {
            let mut world = AnvilWorld::open(&dir, &reg);
            let loaded = world.load_chunk(0, 0).unwrap().unwrap();
            assert_eq!(loaded.get_block_state_id(2, 3, 4), 10);
            assert!(world.load_chunk(5, 5).unwrap().is_none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
