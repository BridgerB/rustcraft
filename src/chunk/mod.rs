//! Chunk storage: bit-packed palettes, block/biome sections, and full columns,
//! with network (de)serialization. Port of typecraft's `chunk` module.

mod biome_section;
mod bit_array;
mod chunk_column;
mod chunk_section;
mod constants;
mod palette_container;

pub use biome_section::BiomeSection;
pub use bit_array::{needed_bits, BitArray};
pub use chunk_column::{ChunkColumn, ChunkColumnOptions, ChunkLightDump};
pub use chunk_section::ChunkSection;
pub use constants::*;
pub use palette_container::{
    read_palette_container, write_palette_container, DirectContainer, IndirectContainer,
    PaletteConfig, PaletteContainer, SingleValueContainer,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn make_column() -> ChunkColumn {
        ChunkColumn::new(ChunkColumnOptions {
            min_y: None,
            world_height: None,
            max_bits_per_block: 15,
            max_bits_per_biome: 7,
        })
    }

    // ── ChunkSection ──

    #[test]
    fn section_create_empty() {
        let section = ChunkSection::new(GLOBAL_BITS_PER_BLOCK, 0);
        assert!(section.is_empty());
        assert_eq!(section.get_block(0, 0, 0), 0);
    }

    #[test]
    fn section_set_get() {
        let mut section = ChunkSection::new(GLOBAL_BITS_PER_BLOCK, 0);
        section.set_block(0, 0, 0, 1);
        assert_eq!(section.get_block(0, 0, 0), 1);
        assert!(!section.is_empty());
    }

    #[test]
    fn section_tracks_solid_count() {
        let mut section = ChunkSection::new(GLOBAL_BITS_PER_BLOCK, 0);
        section.set_block(0, 0, 0, 1);
        section.set_block(1, 0, 0, 2);
        assert_eq!(section.solid_block_count, 2);
        section.set_block(0, 0, 0, 0);
        assert_eq!(section.solid_block_count, 1);
    }

    #[test]
    fn section_palette_upgrade() {
        let mut section = ChunkSection::new(GLOBAL_BITS_PER_BLOCK, 0);
        for i in 0..16u32 {
            section.set_block(i as usize, 0, 0, i + 1);
        }
        for i in 0..16u32 {
            assert_eq!(section.get_block(i as usize, 0, 0), i + 1);
        }
    }

    #[test]
    fn section_direct_palette() {
        let mut section = ChunkSection::new(GLOBAL_BITS_PER_BLOCK, 0);
        for x in 0..16 {
            for z in 0..16 {
                section.set_block(x, 0, z, (x * 16 + z + 1) as u32);
            }
        }
        for x in 0..16 {
            for z in 0..16 {
                assert_eq!(section.get_block(x, 0, z), (x * 16 + z + 1) as u32);
            }
        }
    }

    #[test]
    fn section_from_local_palette() {
        let mut data = BitArray::new(4, 4096);
        data.set(0, 0);
        data.set(1, 1);
        data.set(2, 2);
        let section = ChunkSection::from_local_palette(vec![0, 10, 20], data);
        assert_eq!(section.get_block(0, 0, 0), 0);
        assert_eq!(section.get_block(1, 0, 0), 10);
        assert_eq!(section.get_block(2, 0, 0), 20);
        assert_eq!(section.solid_block_count, 2);
    }

    // ── BiomeSection ──

    #[test]
    fn biome_default_and_set() {
        let mut section = BiomeSection::new(0);
        assert_eq!(section.get_biome(0, 0, 0), 0);
        section.set_biome(0, 0, 0, 5);
        assert_eq!(section.get_biome(0, 0, 0), 5);
    }

    #[test]
    fn biome_from_local_palette() {
        let mut data = BitArray::new(1, 64);
        data.set(0, 0);
        data.set(1, 1);
        let section = BiomeSection::from_local_palette(vec![1, 7], data);
        assert_eq!(section.get_biome(0, 0, 0), 1);
        assert_eq!(section.get_biome(1, 0, 0), 7);
    }

    // ── ChunkColumn ──

    #[test]
    fn column_dimensions() {
        let col = make_column();
        assert_eq!(col.min_y, -64);
        assert_eq!(col.world_height, 384);
        assert_eq!(col.num_sections, 24);
        assert_eq!(col.sections.len(), 24);
        assert_eq!(col.biomes.len(), 24);
    }

    #[test]
    fn column_block_state_ids() {
        let mut col = make_column();
        col.set_block_state_id(0, 0, 0, 1);
        assert_eq!(col.get_block_state_id(0, 0, 0), 1);
    }

    #[test]
    fn column_negative_y() {
        let mut col = make_column();
        col.set_block_state_id(5, -64, 5, 42);
        assert_eq!(col.get_block_state_id(5, -64, 5), 42);
    }

    #[test]
    fn column_positive_y() {
        let mut col = make_column();
        col.set_block_state_id(0, 319, 0, 99);
        assert_eq!(col.get_block_state_id(0, 319, 0), 99);
    }

    #[test]
    fn column_biome_ids() {
        let mut col = make_column();
        col.set_biome_id(0, 0, 0, 5);
        assert_eq!(col.get_biome_id(0, 0, 0), 5);
    }

    #[test]
    fn column_block_light() {
        let mut col = make_column();
        assert_eq!(col.get_block_light(0, 0, 0), 0);
        col.set_block_light(0, 0, 0, 15);
        assert_eq!(col.get_block_light(0, 0, 0), 15);
    }

    #[test]
    fn column_sky_light() {
        let mut col = make_column();
        assert_eq!(col.get_sky_light(0, 0, 0), 0);
        col.set_sky_light(0, 0, 0, 12);
        assert_eq!(col.get_sky_light(0, 0, 0), 12);
    }

    #[test]
    fn column_skips_light_zero_on_null_section() {
        let mut col = make_column();
        col.set_block_light(0, 0, 0, 0);
        assert!(col.block_light_sections[1].is_none());
    }

    #[test]
    fn column_block_entities() {
        use crate::nbt::{compound, nbt_string};
        let mut col = make_column();
        let entity = compound(vec![("id", nbt_string("minecraft:chest"))]);
        col.set_block_entity(5, 10, 3, entity.clone());
        assert_eq!(col.get_block_entity(5, 10, 3), Some(&entity));
        col.remove_block_entity(5, 10, 3);
        assert_eq!(col.get_block_entity(5, 10, 3), None);
    }

    // ── Network roundtrip ──

    #[test]
    fn roundtrips_empty_chunk() {
        let col = make_column();
        let data = col.dump(false);
        let mut col2 = make_column();
        col2.load(&data, false);
        for x in 0..16 {
            for z in 0..16 {
                assert_eq!(col2.get_block_state_id(x, 0, z), 0);
            }
        }
    }

    #[test]
    fn roundtrips_chunk_with_blocks() {
        let mut col = make_column();
        col.set_block_state_id(0, 0, 0, 1);
        col.set_block_state_id(5, 100, 5, 42);
        col.set_block_state_id(15, -64, 15, 999);
        col.set_biome_id(0, 0, 0, 3);

        let data = col.dump(false);
        let mut col2 = make_column();
        col2.load(&data, false);

        assert_eq!(col2.get_block_state_id(0, 0, 0), 1);
        assert_eq!(col2.get_block_state_id(5, 100, 5), 42);
        assert_eq!(col2.get_block_state_id(15, -64, 15), 999);
        assert_eq!(col2.get_biome_id(0, 0, 0), 3);
    }
}
