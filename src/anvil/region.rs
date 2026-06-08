//! Anvil region file (`.mca`) reader/writer. 32×32 chunks per region, stored in
//! 4 KiB sectors with an offset table and timestamp table in the first two.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::nbt::{self, NbtFormat, NbtRoot};

const SECTOR_BYTES: u64 = 4096;
const SECTOR_INTS: usize = (SECTOR_BYTES / 4) as usize;
const CHUNK_HEADER_SIZE: usize = 5;
const VERSION_GZIP: u8 = 1;
const VERSION_DEFLATE: u8 = 2;

pub struct RegionFile {
    pub file_name: PathBuf,
    file: File,
    offsets: Vec<u32>,
    chunk_timestamps: Vec<u32>,
    sector_free: Vec<bool>,
}

fn read_u32_be(buf: &[u8], i: usize) -> u32 {
    u32::from_be_bytes(buf[i..i + 4].try_into().unwrap())
}

impl RegionFile {
    /// Open or create a region file.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<RegionFile> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        let mut size = file.metadata()?.len();
        if size < SECTOR_BYTES {
            let empty = vec![0u8; SECTOR_BYTES as usize];
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&empty)?;
            file.write_all(&empty)?;
            size = SECTOR_BYTES * 2;
        }
        if size & 0xfff != 0 {
            let remaining = SECTOR_BYTES - (size & 0xfff);
            file.seek(SeekFrom::Start(size))?;
            file.write_all(&vec![0u8; remaining as usize])?;
            size += remaining;
        }

        let n_sectors = (size / SECTOR_BYTES) as usize;
        let mut sector_free = vec![true; n_sectors];
        sector_free[0] = false;
        sector_free[1] = false;

        let mut offset_buf = vec![0u8; SECTOR_BYTES as usize];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut offset_buf)?;
        let mut offsets = vec![0u32; SECTOR_INTS];
        for i in 0..SECTOR_INTS {
            let offset = read_u32_be(&offset_buf, i * 4);
            offsets[i] = offset;
            let sector = (offset >> 8) as usize;
            let count = (offset & 0xff) as usize;
            if offset != 0 && sector + count <= sector_free.len() {
                for s in 0..count {
                    sector_free[sector + s] = false;
                }
            }
        }

        let mut ts_buf = vec![0u8; SECTOR_BYTES as usize];
        file.seek(SeekFrom::Start(SECTOR_BYTES))?;
        file.read_exact(&mut ts_buf)?;
        let chunk_timestamps = (0..SECTOR_INTS)
            .map(|i| read_u32_be(&ts_buf, i * 4))
            .collect();

        Ok(RegionFile {
            file_name: path,
            file,
            offsets,
            chunk_timestamps,
            sector_free,
        })
    }

    pub fn has_chunk(&self, x: usize, z: usize) -> bool {
        self.offsets.get(x + z * 32).copied().unwrap_or(0) != 0
    }

    /// Read a chunk's NBT, or `None` if not present.
    pub fn read_chunk(&mut self, x: usize, z: usize) -> std::io::Result<Option<NbtRoot>> {
        let offset = self.offsets.get(x + z * 32).copied().unwrap_or(0);
        if offset == 0 {
            return Ok(None);
        }
        let sector_number = (offset >> 8) as u64;
        let num_sectors = (offset & 0xff) as u64;
        if (sector_number + num_sectors) as usize > self.sector_free.len() {
            return Ok(None);
        }

        self.file
            .seek(SeekFrom::Start(sector_number * SECTOR_BYTES))?;
        let mut header = [0u8; 5];
        self.file.read_exact(&mut header)?;
        let length = read_u32_be(&header, 0) as usize;
        if length <= 1 || length > (SECTOR_BYTES * num_sectors) as usize {
            return Ok(None);
        }
        let version = header[4];

        let mut data = vec![0u8; length - 1];
        self.file.read_exact(&mut data)?;

        let mut decompressed = Vec::new();
        match version {
            VERSION_GZIP => {
                GzDecoder::new(&data[..]).read_to_end(&mut decompressed)?;
            }
            VERSION_DEFLATE => {
                ZlibDecoder::new(&data[..]).read_to_end(&mut decompressed)?;
            }
            other => {
                return Err(std::io::Error::other(format!(
                    "Unknown compression version: {other}"
                )))
            }
        }

        let parsed = nbt::parse_nbt(&decompressed)
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .parsed;
        Ok(Some(parsed))
    }

    /// Write a chunk's NBT, allocating/relocating sectors as needed.
    pub fn write_chunk(&mut self, x: usize, z: usize, nbt_data: &NbtRoot) -> std::io::Result<()> {
        let uncompressed = nbt::write_root(nbt_data, NbtFormat::Big);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&uncompressed)?;
        let compressed = encoder.finish()?;

        let length = compressed.len() + 1;
        let offset = self.offsets[x + z * 32];
        let mut sector_number = (offset >> 8) as usize;
        let sectors_allocated = (offset & 0xff) as usize;
        let sectors_needed = (length + CHUNK_HEADER_SIZE) / SECTOR_BYTES as usize + 1;

        if sectors_needed >= 256 {
            return Err(std::io::Error::other("Chunk data too large (max 1MB)"));
        }

        if sector_number != 0 && sectors_allocated == sectors_needed {
            self.write_chunk_data(sector_number, &compressed, length)?;
        } else {
            for i in 0..sectors_allocated {
                self.sector_free[sector_number + i] = true;
            }
            let run = self.find_free_run(sectors_needed);
            if let Some(run_start) = run {
                sector_number = run_start;
                self.set_offset(x, z, ((sector_number as u32) << 8) | sectors_needed as u32)?;
                for i in 0..sectors_needed {
                    self.sector_free[sector_number + i] = false;
                }
                self.write_chunk_data(sector_number, &compressed, length)?;
            } else {
                let size = self.file.metadata()?.len();
                sector_number = self.sector_free.len();
                let to_grow = sectors_needed * SECTOR_BYTES as usize;
                self.file.seek(SeekFrom::Start(size))?;
                self.file.write_all(&vec![0u8; to_grow])?;
                for _ in 0..sectors_needed {
                    self.sector_free.push(false);
                }
                self.write_chunk_data(sector_number, &compressed, length)?;
                self.set_offset(x, z, ((sector_number as u32) << 8) | sectors_needed as u32)?;
            }
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        self.set_timestamp(x, z, now)
    }

    fn find_free_run(&self, needed: usize) -> Option<usize> {
        let mut run_start = 0;
        let mut run_length = 0;
        for i in 0..self.sector_free.len() {
            if self.sector_free[i] {
                if run_length == 0 {
                    run_start = i;
                }
                run_length += 1;
                if run_length >= needed {
                    return Some(run_start);
                }
            } else {
                run_length = 0;
            }
        }
        None
    }

    fn write_chunk_data(
        &mut self,
        sector_number: usize,
        data: &[u8],
        length: usize,
    ) -> std::io::Result<()> {
        let mut buf = Vec::with_capacity(5 + data.len());
        buf.extend_from_slice(&(length as u32).to_be_bytes());
        buf.push(VERSION_DEFLATE);
        buf.extend_from_slice(data);
        self.file
            .seek(SeekFrom::Start(sector_number as u64 * SECTOR_BYTES))?;
        self.file.write_all(&buf)
    }

    fn set_offset(&mut self, x: usize, z: usize, offset: u32) -> std::io::Result<()> {
        self.offsets[x + z * 32] = offset;
        self.file.seek(SeekFrom::Start(((x + z * 32) * 4) as u64))?;
        self.file.write_all(&offset.to_be_bytes())
    }

    fn set_timestamp(&mut self, x: usize, z: usize, value: u32) -> std::io::Result<()> {
        self.chunk_timestamps[x + z * 32] = value;
        self.file
            .seek(SeekFrom::Start(SECTOR_BYTES + ((x + z * 32) * 4) as u64))?;
        self.file.write_all(&value.to_be_bytes())
    }
}
