//! NBT (Named Binary Tag) reader/writer.
//!
//! Port of typecraft's `nbt` module. Supports three wire formats:
//! `big` (Java disk/legacy network, big-endian), `little` (Bedrock disk,
//! little-endian), and `littleVarint` (Bedrock network, zig-zag varints).
//!
//! 64-bit longs are modeled as `i64` directly (typecraft uses `[high, low]`
//! 32-bit pairs to work around JS number precision — unnecessary in Rust).

use std::io::Read;

// ─── Format ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NbtFormat {
    Big,
    Little,
    LittleVarint,
}

// ─── Tag type identifiers ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NbtType {
    End,
    Byte,
    Short,
    Int,
    Long,
    Float,
    Double,
    ByteArray,
    String,
    List,
    Compound,
    IntArray,
    LongArray,
}

impl NbtType {
    pub fn id(self) -> u8 {
        match self {
            NbtType::End => 0,
            NbtType::Byte => 1,
            NbtType::Short => 2,
            NbtType::Int => 3,
            NbtType::Long => 4,
            NbtType::Float => 5,
            NbtType::Double => 6,
            NbtType::ByteArray => 7,
            NbtType::String => 8,
            NbtType::List => 9,
            NbtType::Compound => 10,
            NbtType::IntArray => 11,
            NbtType::LongArray => 12,
        }
    }

    pub fn from_id(id: u8) -> Option<NbtType> {
        Some(match id {
            0 => NbtType::End,
            1 => NbtType::Byte,
            2 => NbtType::Short,
            3 => NbtType::Int,
            4 => NbtType::Long,
            5 => NbtType::Float,
            6 => NbtType::Double,
            7 => NbtType::ByteArray,
            8 => NbtType::String,
            9 => NbtType::List,
            10 => NbtType::Compound,
            11 => NbtType::IntArray,
            12 => NbtType::LongArray,
            _ => return None,
        })
    }
}

// ─── Tag ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum NbtTag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    ByteArray(Vec<i8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    List(NbtList),
    Compound(NbtCompound),
}

impl NbtTag {
    pub fn nbt_type(&self) -> NbtType {
        match self {
            NbtTag::Byte(_) => NbtType::Byte,
            NbtTag::Short(_) => NbtType::Short,
            NbtTag::Int(_) => NbtType::Int,
            NbtTag::Long(_) => NbtType::Long,
            NbtTag::Float(_) => NbtType::Float,
            NbtTag::Double(_) => NbtType::Double,
            NbtTag::String(_) => NbtType::String,
            NbtTag::ByteArray(_) => NbtType::ByteArray,
            NbtTag::IntArray(_) => NbtType::IntArray,
            NbtTag::LongArray(_) => NbtType::LongArray,
            NbtTag::List(_) => NbtType::List,
            NbtTag::Compound(_) => NbtType::Compound,
        }
    }

    pub fn as_compound(&self) -> Option<&NbtCompound> {
        match self {
            NbtTag::Compound(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&NbtList> {
        match self {
            NbtTag::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&str> {
        match self {
            NbtTag::String(s) => Some(s),
            _ => None,
        }
    }
}

/// A homogeneous list of payloads, all of element type `ty`. Each item in
/// `items` is an `NbtTag` of that type (empty for an `End`-typed list).
#[derive(Debug, Clone, PartialEq)]
pub struct NbtList {
    pub ty: NbtType,
    pub items: Vec<NbtTag>,
}

impl NbtList {
    pub fn empty() -> Self {
        NbtList {
            ty: NbtType::End,
            items: Vec::new(),
        }
    }
}

/// An insertion-ordered compound (NBT compounds preserve key order on disk).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NbtCompound {
    entries: Vec<(String, NbtTag)>,
}

impl NbtCompound {
    pub fn new() -> Self {
        NbtCompound {
            entries: Vec::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&NbtTag> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: impl Into<String>, tag: NbtTag) {
        let key = key.into();
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = tag;
        } else {
            self.entries.push((key, tag));
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &NbtTag)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl FromIterator<(String, NbtTag)> for NbtCompound {
    fn from_iter<T: IntoIterator<Item = (String, NbtTag)>>(iter: T) -> Self {
        NbtCompound {
            entries: iter.into_iter().collect(),
        }
    }
}

/// A root NBT compound carrying a name (the named top-level tag).
#[derive(Debug, Clone, PartialEq)]
pub struct NbtRoot {
    pub name: String,
    pub value: NbtCompound,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NbtParseResult {
    pub parsed: NbtRoot,
    pub format: NbtFormat,
    pub bytes_read: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NbtError {
    UnexpectedEof,
    UnknownTagId(u8),
    ExpectedCompound(u8),
    NegativeCount(i32),
    BadFormat,
}

impl std::fmt::Display for NbtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NbtError::UnexpectedEof => write!(f, "NBT: unexpected end of buffer"),
            NbtError::UnknownTagId(id) => write!(f, "NBT: unknown tag ID {id}"),
            NbtError::ExpectedCompound(id) => {
                write!(f, "NBT: expected compound tag (10), got {id}")
            }
            NbtError::NegativeCount(n) => write!(f, "NBT: negative array count {n}"),
            NbtError::BadFormat => write!(f, "NBT: could not detect format"),
        }
    }
}

impl std::error::Error for NbtError {}

type Res<T> = Result<T, NbtError>;

// ─── Builder functions ──────────────────────────────────────────────────────

pub fn nbt_byte(value: i8) -> NbtTag {
    NbtTag::Byte(value)
}
pub fn nbt_short(value: i16) -> NbtTag {
    NbtTag::Short(value)
}
pub fn nbt_int(value: i32) -> NbtTag {
    NbtTag::Int(value)
}
pub fn nbt_long(value: i64) -> NbtTag {
    NbtTag::Long(value)
}
pub fn nbt_float(value: f32) -> NbtTag {
    NbtTag::Float(value)
}
pub fn nbt_double(value: f64) -> NbtTag {
    NbtTag::Double(value)
}
pub fn nbt_string(value: impl Into<String>) -> NbtTag {
    NbtTag::String(value.into())
}
pub fn nbt_byte_array(value: Vec<i8>) -> NbtTag {
    NbtTag::ByteArray(value)
}
pub fn nbt_int_array(value: Vec<i32>) -> NbtTag {
    NbtTag::IntArray(value)
}
pub fn nbt_long_array(value: Vec<i64>) -> NbtTag {
    NbtTag::LongArray(value)
}
pub fn nbt_list(ty: NbtType, items: Vec<NbtTag>) -> NbtTag {
    NbtTag::List(NbtList { ty, items })
}
pub fn nbt_bool(value: bool) -> NbtTag {
    NbtTag::Short(if value { 1 } else { 0 })
}

/// Build a (nested) compound tag from `(key, tag)` pairs.
pub fn nbt_compound<K: Into<String>>(pairs: Vec<(K, NbtTag)>) -> NbtTag {
    NbtTag::Compound(compound(pairs))
}

/// Build a named root compound.
pub fn nbt_root<K: Into<String>>(name: impl Into<String>, pairs: Vec<(K, NbtTag)>) -> NbtRoot {
    NbtRoot {
        name: name.into(),
        value: compound(pairs),
    }
}

/// Build an `NbtCompound` from `(key, tag)` pairs, preserving order.
pub fn compound<K: Into<String>>(pairs: Vec<(K, NbtTag)>) -> NbtCompound {
    pairs.into_iter().map(|(k, v)| (k.into(), v)).collect()
}

// ─── Simplify (strip type wrappers) ─────────────────────────────────────────

/// An untyped view of NBT, mirroring typecraft's `simplifyNbt`.
#[derive(Debug, Clone, PartialEq)]
pub enum Simple {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    ByteArray(Vec<i8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    List(Vec<Simple>),
    Compound(Vec<(String, Simple)>),
}

pub fn simplify(tag: &NbtTag) -> Simple {
    match tag {
        NbtTag::Byte(v) => Simple::Byte(*v),
        NbtTag::Short(v) => Simple::Short(*v),
        NbtTag::Int(v) => Simple::Int(*v),
        NbtTag::Long(v) => Simple::Long(*v),
        NbtTag::Float(v) => Simple::Float(*v),
        NbtTag::Double(v) => Simple::Double(*v),
        NbtTag::String(v) => Simple::String(v.clone()),
        NbtTag::ByteArray(v) => Simple::ByteArray(v.clone()),
        NbtTag::IntArray(v) => Simple::IntArray(v.clone()),
        NbtTag::LongArray(v) => Simple::LongArray(v.clone()),
        NbtTag::List(list) => Simple::List(list.items.iter().map(simplify).collect()),
        NbtTag::Compound(c) => {
            Simple::Compound(c.iter().map(|(k, v)| (k.clone(), simplify(v))).collect())
        }
    }
}

// ─── Equality (order-insensitive for compounds, matching typecraft) ─────────

pub fn equal_nbt(a: &NbtTag, b: &NbtTag) -> bool {
    match (a, b) {
        (NbtTag::Compound(ca), NbtTag::Compound(cb)) => {
            ca.len() == cb.len()
                && ca
                    .iter()
                    .all(|(k, v)| cb.get(k).map(|bv| equal_nbt(v, bv)).unwrap_or(false))
        }
        (NbtTag::List(la), NbtTag::List(lb)) => {
            la.ty == lb.ty
                && la.items.len() == lb.items.len()
                && la.items.iter().zip(&lb.items).all(|(x, y)| equal_nbt(x, y))
        }
        _ => a == b,
    }
}

// ─── Reader ──────────────────────────────────────────────────────────────────

// The fixed-width numeric read/write methods differ only by type, byte count, and
// (for the integers) whether littleVarint zig-zags instead of writing raw LE. Generate
// them so the three wire formats aren't triplicated by hand. `_zz` variants take a
// zig-zag method name for the littleVarint arm; the plain variants treat
// littleVarint == little (raw LE), which is correct for i16/f32/f64.
macro_rules! nbt_read_num {
    ($name:ident, $ty:ty, $n:literal) => {
        fn $name(&mut self) -> Res<$ty> {
            let arr: [u8; $n] = self.take($n)?.try_into().unwrap();
            Ok(match self.format {
                NbtFormat::Big => <$ty>::from_be_bytes(arr),
                _ => <$ty>::from_le_bytes(arr),
            })
        }
    };
    ($name:ident, $ty:ty, $n:literal, $zigzag:ident) => {
        fn $name(&mut self) -> Res<$ty> {
            match self.format {
                NbtFormat::LittleVarint => self.$zigzag(),
                _ => {
                    let arr: [u8; $n] = self.take($n)?.try_into().unwrap();
                    Ok(match self.format {
                        NbtFormat::Big => <$ty>::from_be_bytes(arr),
                        _ => <$ty>::from_le_bytes(arr),
                    })
                }
            }
        }
    };
}
macro_rules! nbt_write_num {
    ($name:ident, $ty:ty) => {
        fn $name(&mut self, value: $ty) {
            match self.format {
                NbtFormat::Big => self.buf.extend_from_slice(&value.to_be_bytes()),
                _ => self.buf.extend_from_slice(&value.to_le_bytes()),
            }
        }
    };
    ($name:ident, $ty:ty, $zigzag:ident) => {
        fn $name(&mut self, value: $ty) {
            match self.format {
                NbtFormat::Big => self.buf.extend_from_slice(&value.to_be_bytes()),
                NbtFormat::Little => self.buf.extend_from_slice(&value.to_le_bytes()),
                NbtFormat::LittleVarint => self.$zigzag(value),
            }
        }
    };
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    format: NbtFormat,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8], offset: usize, format: NbtFormat) -> Self {
        Reader {
            buf,
            pos: offset,
            format,
        }
    }

    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(NbtError::UnexpectedEof)?;
        let slice = self.buf.get(self.pos..end).ok_or(NbtError::UnexpectedEof)?;
        self.pos = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Res<u8> {
        Ok(self.take(1)?[0])
    }

    fn read_i8(&mut self) -> Res<i8> {
        Ok(self.take(1)?[0] as i8)
    }

    nbt_read_num!(read_i16, i16, 2);
    nbt_read_num!(read_i32, i32, 4, read_zigzag32);
    nbt_read_num!(read_i64, i64, 8, read_zigzag64);
    nbt_read_num!(read_f32, f32, 4);
    nbt_read_num!(read_f64, f64, 8);

    /// Unsigned LEB128 (used by littleVarint for lengths and as zig-zag base).
    fn read_uvarint(&mut self) -> Res<u64> {
        let mut value: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.read_u8()?;
            value |= ((byte & 0x7f) as u64).wrapping_shl(shift);
            shift += 7;
            if byte & 0x80 == 0 {
                break;
            }
        }
        Ok(value)
    }

    fn read_zigzag32(&mut self) -> Res<i32> {
        let raw = self.read_uvarint()? as u32;
        Ok(((raw >> 1) as i32) ^ -((raw & 1) as i32))
    }

    fn read_zigzag64(&mut self) -> Res<i64> {
        let raw = self.read_uvarint()?;
        Ok(((raw >> 1) as i64) ^ -((raw & 1) as i64))
    }

    fn read_string(&mut self) -> Res<String> {
        let len = match self.format {
            NbtFormat::Big => {
                let b = self.take(2)?;
                u16::from_be_bytes([b[0], b[1]]) as usize
            }
            NbtFormat::Little => {
                let b = self.take(2)?;
                u16::from_le_bytes([b[0], b[1]]) as usize
            }
            NbtFormat::LittleVarint => self.read_uvarint()? as usize,
        };
        let bytes = self.take(len)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn read_array_count(&mut self) -> Res<usize> {
        // An array/list count is a plain i32 in every format (zig-zag in littleVarint),
        // i.e. exactly read_i32 — but it must be non-negative.
        let count = self.read_i32()?;
        if count < 0 {
            return Err(NbtError::NegativeCount(count));
        }
        Ok(count as usize)
    }

    fn read_payload(&mut self, ty: NbtType) -> Res<NbtTag> {
        Ok(match ty {
            NbtType::End => return Err(NbtError::UnknownTagId(0)),
            NbtType::Byte => NbtTag::Byte(self.read_i8()?),
            NbtType::Short => NbtTag::Short(self.read_i16()?),
            NbtType::Int => NbtTag::Int(self.read_i32()?),
            NbtType::Long => NbtTag::Long(self.read_i64()?),
            NbtType::Float => NbtTag::Float(self.read_f32()?),
            NbtType::Double => NbtTag::Double(self.read_f64()?),
            NbtType::String => NbtTag::String(self.read_string()?),
            NbtType::ByteArray => {
                let count = self.read_array_count()?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(self.read_i8()?);
                }
                NbtTag::ByteArray(v)
            }
            NbtType::IntArray => {
                let count = self.read_array_count()?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(self.read_i32()?);
                }
                NbtTag::IntArray(v)
            }
            NbtType::LongArray => {
                let count = self.read_array_count()?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(self.read_i64()?);
                }
                NbtTag::LongArray(v)
            }
            NbtType::List => NbtTag::List(self.read_list()?),
            NbtType::Compound => NbtTag::Compound(self.read_compound()?),
        })
    }

    fn read_list(&mut self) -> Res<NbtList> {
        let ty = NbtType::from_id(self.read_u8()?).ok_or(NbtError::UnknownTagId(0))?;
        let count = self.read_array_count()?;
        if ty == NbtType::End {
            // An end-typed list carries no payloads.
            return Ok(NbtList {
                ty,
                items: Vec::new(),
            });
        }
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(self.read_payload(ty)?);
        }
        Ok(NbtList { ty, items })
    }

    fn read_compound(&mut self) -> Res<NbtCompound> {
        let mut c = NbtCompound::new();
        loop {
            let id = self.read_u8()?;
            if id == 0 {
                break;
            }
            let ty = NbtType::from_id(id).ok_or(NbtError::UnknownTagId(id))?;
            let name = self.read_string()?;
            let payload = self.read_payload(ty)?;
            c.insert(name, payload);
        }
        Ok(c)
    }
}

// ─── Writer ──────────────────────────────────────────────────────────────────

struct Writer {
    buf: Vec<u8>,
    format: NbtFormat,
}

impl Writer {
    fn new(format: NbtFormat) -> Self {
        Writer {
            buf: Vec::new(),
            format,
        }
    }

    nbt_write_num!(write_i16, i16);
    nbt_write_num!(write_i32, i32, write_zigzag32);
    nbt_write_num!(write_i64, i64, write_zigzag64);
    nbt_write_num!(write_f32, f32);
    nbt_write_num!(write_f64, f64);

    fn write_uvarint(&mut self, mut v: u64) {
        while v > 0x7f {
            self.buf.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }

    fn write_zigzag32(&mut self, value: i32) {
        let zz = (value.wrapping_shl(1) ^ (value >> 31)) as u32;
        self.write_uvarint(zz as u64);
    }

    fn write_zigzag64(&mut self, value: i64) {
        let zz = (value.wrapping_shl(1) ^ (value >> 63)) as u64;
        self.write_uvarint(zz);
    }

    fn write_string(&mut self, value: &str) {
        let bytes = value.as_bytes();
        match self.format {
            NbtFormat::Big => self
                .buf
                .extend_from_slice(&(bytes.len() as u16).to_be_bytes()),
            NbtFormat::Little => self
                .buf
                .extend_from_slice(&(bytes.len() as u16).to_le_bytes()),
            NbtFormat::LittleVarint => self.write_uvarint(bytes.len() as u64),
        }
        self.buf.extend_from_slice(bytes);
    }

    fn write_array_count(&mut self, count: usize) {
        // Mirror of read_array_count: an array/list count is written exactly as an i32.
        self.write_i32(count as i32);
    }

    fn write_payload(&mut self, tag: &NbtTag) {
        match tag {
            NbtTag::Byte(v) => self.buf.push(*v as u8),
            NbtTag::Short(v) => self.write_i16(*v),
            NbtTag::Int(v) => self.write_i32(*v),
            NbtTag::Long(v) => self.write_i64(*v),
            NbtTag::Float(v) => self.write_f32(*v),
            NbtTag::Double(v) => self.write_f64(*v),
            NbtTag::String(v) => self.write_string(v),
            NbtTag::ByteArray(v) => {
                self.write_array_count(v.len());
                for b in v {
                    self.buf.push(*b as u8);
                }
            }
            NbtTag::IntArray(v) => {
                self.write_array_count(v.len());
                for i in v {
                    self.write_i32(*i);
                }
            }
            NbtTag::LongArray(v) => {
                self.write_array_count(v.len());
                for l in v {
                    self.write_i64(*l);
                }
            }
            NbtTag::List(list) => {
                self.buf.push(list.ty.id());
                self.write_array_count(list.items.len());
                for item in &list.items {
                    self.write_payload(item);
                }
            }
            NbtTag::Compound(c) => self.write_compound(c),
        }
    }

    fn write_compound(&mut self, c: &NbtCompound) {
        for (name, tag) in c.iter() {
            self.buf.push(tag.nbt_type().id());
            self.write_string(name);
            self.write_payload(tag);
        }
        self.buf.push(0);
    }
}

// ─── Decompression helpers ──────────────────────────────────────────────────

fn has_gzip_header(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b
}

fn has_bedrock_level_header(data: &[u8]) -> bool {
    data.len() >= 4 && data[1] == 0 && data[2] == 0 && data[3] == 0
}

fn decompress(data: &[u8]) -> Vec<u8> {
    if has_gzip_header(data) {
        let mut out = Vec::new();
        if flate2::read::GzDecoder::new(data)
            .read_to_end(&mut out)
            .is_ok()
        {
            return out;
        }
    }
    let mut out = Vec::new();
    if flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .is_ok()
        && !out.is_empty()
    {
        return out;
    }
    data.to_vec()
}

// ─── Public read API ─────────────────────────────────────────────────────────

/// Read a named root compound at `offset`. Returns `(root, bytes_read)`.
pub fn read_root(buf: &[u8], offset: usize, format: NbtFormat) -> Res<(NbtRoot, usize)> {
    let mut r = Reader::new(buf, offset, format);
    let id = r.read_u8()?;
    if id != NbtType::Compound.id() {
        return Err(NbtError::ExpectedCompound(id));
    }
    let name = r.read_string()?;
    let value = r.read_compound()?;
    Ok((NbtRoot { name, value }, r.pos - offset))
}

/// Read an anonymous (nameless, network) NBT tag at `offset`.
/// Returns `(None, 1)` for an `End` (absent) tag.
pub fn read_anonymous(
    buf: &[u8],
    offset: usize,
    format: NbtFormat,
) -> Res<(Option<NbtRoot>, usize)> {
    let mut r = Reader::new(buf, offset, format);
    let id = r.read_u8()?;
    if id == 0 {
        return Ok((None, 1));
    }
    let ty = NbtType::from_id(id).ok_or(NbtError::UnknownTagId(id))?;
    // Network text components may be a bare non-compound tag (e.g. a string).
    // Wrap those under the "" key so consumers can read them uniformly.
    let value = if ty == NbtType::Compound {
        r.read_compound()?
    } else {
        let payload = r.read_payload(ty)?;
        let mut c = NbtCompound::new();
        c.insert("", payload);
        c
    };
    Ok((
        Some(NbtRoot {
            name: String::new(),
            value,
        }),
        r.pos - offset,
    ))
}

/// Parse with explicit format, no decompression.
pub fn parse_uncompressed(data: &[u8], format: NbtFormat) -> Res<NbtRoot> {
    Ok(read_root(data, 0, format)?.0)
}

/// Parse with auto-detection: decompress (gzip/zlib) then try each format.
pub fn parse_nbt(data: &[u8]) -> Res<NbtParseResult> {
    let decompressed = decompress(data);

    if has_bedrock_level_header(&decompressed) {
        let (parsed, size) = read_root(&decompressed, 8, NbtFormat::Little)?;
        return Ok(NbtParseResult {
            parsed,
            format: NbtFormat::Little,
            bytes_read: size,
        });
    }

    for format in [NbtFormat::Big, NbtFormat::Little, NbtFormat::LittleVarint] {
        if let Ok((parsed, size)) = read_root(&decompressed, 0, format) {
            return Ok(NbtParseResult {
                parsed,
                format,
                bytes_read: size,
            });
        }
    }
    Err(NbtError::BadFormat)
}

// ─── Public write API ────────────────────────────────────────────────────────

/// Write a named root compound.
pub fn write_root(root: &NbtRoot, format: NbtFormat) -> Vec<u8> {
    let mut w = Writer::new(format);
    w.buf.push(NbtType::Compound.id());
    w.write_string(&root.name);
    w.write_compound(&root.value);
    w.buf
}

/// Write an anonymous (nameless, network) NBT tag. `None` writes a single
/// `End` byte.
pub fn write_anonymous(root: Option<&NbtRoot>, format: NbtFormat) -> Vec<u8> {
    let Some(root) = root else {
        return vec![0];
    };
    let mut w = Writer::new(format);
    w.buf.push(NbtType::Compound.id());
    w.write_compound(&root.value);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── builder functions ──────────────────────────────────────────────────

    #[test]
    fn builders() {
        assert_eq!(nbt_byte(42), NbtTag::Byte(42));
        assert_eq!(nbt_short(1000), NbtTag::Short(1000));
        assert_eq!(nbt_int(100000), NbtTag::Int(100000));
        assert_eq!(nbt_long(42), NbtTag::Long(42));
        assert_eq!(nbt_float(3.14), NbtTag::Float(3.14));
        assert_eq!(nbt_double(1.23456), NbtTag::Double(1.23456));
        assert_eq!(nbt_string("hello"), NbtTag::String("hello".into()));
        assert_eq!(
            nbt_byte_array(vec![1, 2, 3]),
            NbtTag::ByteArray(vec![1, 2, 3])
        );
        assert_eq!(nbt_int_array(vec![10, 20]), NbtTag::IntArray(vec![10, 20]));
        assert_eq!(nbt_long_array(vec![1, 2]), NbtTag::LongArray(vec![1, 2]));
        assert_eq!(nbt_bool(true), NbtTag::Short(1));
        assert_eq!(nbt_bool(false), NbtTag::Short(0));
    }

    #[test]
    fn builds_compound_and_list() {
        let tag = nbt_compound(vec![("x", nbt_int(5))]);
        let c = tag.as_compound().unwrap();
        assert_eq!(c.get("x"), Some(&NbtTag::Int(5)));

        let list = nbt_list(NbtType::Int, vec![nbt_int(1), nbt_int(2), nbt_int(3)]);
        let l = list.as_list().unwrap();
        assert_eq!(l.ty, NbtType::Int);
        assert_eq!(l.items.len(), 3);

        let empty = NbtTag::List(NbtList::empty());
        assert_eq!(empty.as_list().unwrap().ty, NbtType::End);
    }

    // ─── roundtrip across all formats ────────────────────────────────────────

    fn sample_root() -> NbtRoot {
        nbt_root(
            "Level",
            vec![
                ("byteTest", nbt_byte(127)),
                ("shortTest", nbt_short(32767)),
                ("intTest", nbt_int(2147483647)),
                ("longTest", nbt_long(9223372036854775807)),
                ("floatTest", nbt_float(0.49823147)),
                ("doubleTest", nbt_double(0.4931287132182315)),
                (
                    "stringTest",
                    nbt_string("HELLO WORLD THIS IS A TEST STRING ÅÄÖ!"),
                ),
                ("byteArrayTest", nbt_byte_array(vec![0, 62, 34, 16, 8])),
                ("intArrayTest", nbt_int_array(vec![-1, 0, 1, 2147483647])),
                ("longArrayTest", nbt_long_array(vec![0, 11, 12, 13, 14, 15])),
                (
                    "listTest (long)",
                    nbt_list(
                        NbtType::Long,
                        vec![nbt_long(11), nbt_long(12), nbt_long(13)],
                    ),
                ),
                (
                    "nested compound test",
                    nbt_compound(vec![
                        ("ham", nbt_compound(vec![("name", nbt_string("Hampus"))])),
                        ("egg", nbt_compound(vec![("name", nbt_string("Eggbert"))])),
                    ]),
                ),
            ],
        )
    }

    fn roundtrip(format: NbtFormat) {
        let original = sample_root();
        let written = write_root(&original, format);
        let reparsed = parse_uncompressed(&written, format).unwrap();
        assert!(
            equal_nbt(
                &NbtTag::Compound(original.value.clone()),
                &NbtTag::Compound(reparsed.value.clone())
            ),
            "roundtrip mismatch for {format:?}"
        );
        assert_eq!(original.name, reparsed.name);
    }

    #[test]
    fn roundtrips_big() {
        roundtrip(NbtFormat::Big);
    }

    #[test]
    fn roundtrips_little() {
        roundtrip(NbtFormat::Little);
    }

    #[test]
    fn roundtrips_little_varint() {
        roundtrip(NbtFormat::LittleVarint);
    }

    #[test]
    fn cross_format_big_little_big() {
        let original = sample_root();
        let as_little = write_root(&original, NbtFormat::Little);
        let from_little = parse_uncompressed(&as_little, NbtFormat::Little).unwrap();
        let back = write_root(&from_little, NbtFormat::Big);
        let final_root = parse_uncompressed(&back, NbtFormat::Big).unwrap();
        assert!(equal_nbt(
            &NbtTag::Compound(original.value),
            &NbtTag::Compound(final_root.value)
        ));
    }

    // ─── auto-detection + decompression ──────────────────────────────────────

    #[test]
    fn detects_uncompressed_big() {
        let data = write_root(&sample_root(), NbtFormat::Big);
        let result = parse_nbt(&data).unwrap();
        assert_eq!(result.format, NbtFormat::Big);
        assert_eq!(result.parsed.name, "Level");
    }

    #[test]
    fn detects_gzip_compressed() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let data = write_root(&sample_root(), NbtFormat::Big);
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&data).unwrap();
        let gz = enc.finish().unwrap();

        let result = parse_nbt(&gz).unwrap();
        assert_eq!(result.format, NbtFormat::Big);
        assert_eq!(
            result.parsed.value.get("shortTest"),
            Some(&NbtTag::Short(32767))
        );
    }

    // ─── simplify ────────────────────────────────────────────────────────────

    #[test]
    fn simplifies_primitives_and_structures() {
        assert_eq!(simplify(&nbt_int(42)), Simple::Int(42));
        assert_eq!(
            simplify(&nbt_string("hello")),
            Simple::String("hello".into())
        );

        let tag = nbt_compound(vec![("x", nbt_int(5)), ("name", nbt_string("test"))]);
        assert_eq!(
            simplify(&tag),
            Simple::Compound(vec![
                ("x".into(), Simple::Int(5)),
                ("name".into(), Simple::String("test".into())),
            ])
        );

        let list = nbt_list(NbtType::Int, vec![nbt_int(10), nbt_int(20), nbt_int(30)]);
        assert_eq!(
            simplify(&list),
            Simple::List(vec![Simple::Int(10), Simple::Int(20), Simple::Int(30)])
        );
    }

    // ─── equal_nbt ───────────────────────────────────────────────────────────

    #[test]
    fn equality() {
        assert!(equal_nbt(&nbt_int(42), &nbt_int(42)));
        assert!(!equal_nbt(&nbt_int(42), &nbt_int(43)));
        assert!(!equal_nbt(&nbt_int(42), &nbt_short(42)));
        assert!(!equal_nbt(&nbt_float(1.0), &nbt_double(1.0)));

        let a = nbt_compound(vec![("x", nbt_int(1))]);
        let b = nbt_compound(vec![("x", nbt_int(1))]);
        let c = nbt_compound(vec![("x", nbt_int(2))]);
        let d = nbt_compound(vec![("y", nbt_int(1))]);
        assert!(equal_nbt(&a, &b));
        assert!(!equal_nbt(&a, &c));
        assert!(!equal_nbt(&a, &d));

        let la = nbt_list(NbtType::Int, vec![nbt_int(1), nbt_int(2), nbt_int(3)]);
        let lc = nbt_list(NbtType::Int, vec![nbt_int(1), nbt_int(2), nbt_int(4)]);
        let ld = nbt_list(NbtType::Byte, vec![nbt_byte(1), nbt_byte(2), nbt_byte(3)]);
        assert!(equal_nbt(&la, &la.clone()));
        assert!(!equal_nbt(&la, &lc));
        assert!(!equal_nbt(&la, &ld));
    }

    #[test]
    fn compound_order_insensitive_equality() {
        let a = nbt_compound(vec![("x", nbt_int(1)), ("y", nbt_int(2))]);
        let b = nbt_compound(vec![("y", nbt_int(2)), ("x", nbt_int(1))]);
        assert!(equal_nbt(&a, &b));
    }

    // ─── edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn handles_empty_list_end_type() {
        let root = nbt_root("", vec![("items", NbtTag::List(NbtList::empty()))]);
        let written = write_root(&root, NbtFormat::Big);
        let reparsed = parse_uncompressed(&written, NbtFormat::Big).unwrap();
        let items = reparsed.value.get("items").unwrap().as_list().unwrap();
        assert_eq!(items.ty, NbtType::End);
        assert_eq!(items.items.len(), 0);
    }

    #[test]
    fn handles_utf8_strings() {
        let root = nbt_root(
            "",
            vec![
                ("jp", nbt_string("こんにちは!")),
                ("nordic", nbt_string("ÅÄÖ")),
            ],
        );
        let written = write_root(&root, NbtFormat::Big);
        let reparsed = parse_uncompressed(&written, NbtFormat::Big).unwrap();
        assert_eq!(
            reparsed.value.get("jp").unwrap().as_string(),
            Some("こんにちは!")
        );
        assert_eq!(
            reparsed.value.get("nordic").unwrap().as_string(),
            Some("ÅÄÖ")
        );
    }

    #[test]
    fn handles_min_max_numeric() {
        let root = nbt_root(
            "",
            vec![
                ("maxByte", nbt_byte(127)),
                ("minByte", nbt_byte(-128)),
                ("maxShort", nbt_short(32767)),
                ("minShort", nbt_short(-32768)),
                ("maxInt", nbt_int(2147483647)),
                ("minInt", nbt_int(-2147483648)),
                ("maxLong", nbt_long(i64::MAX)),
                ("minLong", nbt_long(i64::MIN)),
            ],
        );
        for format in [NbtFormat::Big, NbtFormat::Little, NbtFormat::LittleVarint] {
            let written = write_root(&root, format);
            let reparsed = parse_uncompressed(&written, format).unwrap();
            assert!(
                equal_nbt(
                    &NbtTag::Compound(root.value.clone()),
                    &NbtTag::Compound(reparsed.value)
                ),
                "min/max roundtrip failed for {format:?}"
            );
        }
    }

    #[test]
    fn anonymous_roundtrip() {
        let root = nbt_root("", vec![("x", nbt_int(42))]);
        let written = write_anonymous(Some(&root), NbtFormat::Big);
        let (reparsed, _) = read_anonymous(&written, 0, NbtFormat::Big).unwrap();
        let reparsed = reparsed.unwrap();
        assert_eq!(reparsed.value.get("x"), Some(&NbtTag::Int(42)));

        let absent = write_anonymous(None, NbtFormat::Big);
        assert_eq!(absent, vec![0]);
        let (none, size) = read_anonymous(&absent, 0, NbtFormat::Big).unwrap();
        assert!(none.is_none());
        assert_eq!(size, 1);
    }
}
