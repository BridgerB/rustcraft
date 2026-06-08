//! `level.dat` reader/writer (gzip-compressed NBT).

use std::io::{Read, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::nbt::{self, compound, nbt_int, nbt_long, nbt_string, NbtFormat, NbtTag};

#[derive(Debug, Clone)]
pub struct LevelData {
    pub level_name: String,
    pub version: String,
    pub generator_name: String,
    pub random_seed: i64,
}

/// Read `level.dat` from a Minecraft world save.
pub fn read_level_dat(path: impl AsRef<Path>) -> std::io::Result<LevelData> {
    let content = std::fs::read(path)?;
    let mut decompressed = Vec::new();
    GzDecoder::new(&content[..]).read_to_end(&mut decompressed)?;
    let parsed = nbt::parse_nbt(&decompressed)
        .map_err(|e| std::io::Error::other(e.to_string()))?
        .parsed;

    let data = match parsed.value.get("Data") {
        Some(NbtTag::Compound(c)) => c,
        _ => return Err(std::io::Error::other("level.dat missing Data compound")),
    };

    let level_name = match data.get("LevelName") {
        Some(NbtTag::String(s)) => s.clone(),
        _ => "Unknown".into(),
    };
    let version = match data.get("Version") {
        Some(NbtTag::Compound(v)) => match v.get("Name") {
            Some(NbtTag::String(s)) => s.clone(),
            _ => "unknown".into(),
        },
        _ => "unknown".into(),
    };
    let generator_name = match data.get("generatorName") {
        Some(NbtTag::String(s)) => s.clone(),
        _ => "default".into(),
    };
    let random_seed = match data.get("RandomSeed") {
        Some(NbtTag::Long(l)) => *l,
        _ => 0,
    };

    Ok(LevelData {
        level_name,
        version,
        generator_name,
        random_seed,
    })
}

/// Write a `level.dat` file.
pub fn write_level_dat(path: impl AsRef<Path>, data: &LevelData) -> std::io::Result<()> {
    let root = nbt::NbtRoot {
        name: String::new(),
        value: compound(vec![(
            "Data",
            NbtTag::Compound(compound(vec![
                (
                    "Version",
                    NbtTag::Compound(compound(vec![("Name", nbt_string(data.version.clone()))])),
                ),
                ("LevelName", nbt_string(data.level_name.clone())),
                ("generatorName", nbt_string(data.generator_name.clone())),
                ("version", nbt_int(19133)),
                ("RandomSeed", nbt_long(data.random_seed)),
            ])),
        )]),
    };

    let uncompressed = nbt::write_root(&root, NbtFormat::Big);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&uncompressed)?;
    let compressed = encoder.finish()?;
    std::fs::write(path, compressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_dat_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rustcraft-level-{}.dat", std::process::id()));
        let data = LevelData {
            level_name: "My World".into(),
            version: "26.1.2".into(),
            generator_name: "default".into(),
            random_seed: 1234567890123,
        };
        write_level_dat(&path, &data).unwrap();
        let read = read_level_dat(&path).unwrap();
        assert_eq!(read.level_name, "My World");
        assert_eq!(read.version, "26.1.2");
        assert_eq!(read.random_seed, 1234567890123);
        let _ = std::fs::remove_file(&path);
    }
}
