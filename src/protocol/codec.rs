//! Schema-driven packet codec — a Rust interpreter of typecraft's ProtoDef
//! schemas. Reads/writes [`PValue`] trees against `serde_json::Value` schemas.
//!
//! Unlike the TS codec there is no `sizeOf`: writes append to a growable `Vec`,
//! so buffers never need pre-sizing.

use std::collections::HashMap;

use serde_json::Value;

use super::value::PValue;
use crate::nbt::{self, NbtFormat};
use crate::varint::{push_var_int, push_var_long, read_var_int, read_var_long};

#[derive(Debug)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "codec: {}", self.0)
    }
}
impl std::error::Error for CodecError {}

type Res<T> = Result<T, CodecError>;

fn err<T>(msg: impl Into<String>) -> Res<T> {
    Err(CodecError(msg.into()))
}

/// A resolved set of named protocol types.
pub struct TypeRegistry {
    types: HashMap<String, Value>,
}

impl TypeRegistry {
    pub fn new(types: HashMap<String, Value>) -> Self {
        TypeRegistry { types }
    }

    fn get(&self, name: &str) -> Option<&Value> {
        self.types.get(name)
    }
}

/// Context chain for `compareTo` path resolution (`../` walks to a parent).
enum Ctx<'a> {
    Root,
    Level {
        map: &'a [(String, PValue)],
        parent: &'a Ctx<'a>,
    },
}

fn resolve_compare_to<'a>(path: &str, ctx: &'a Ctx<'a>) -> Option<&'a PValue> {
    let mut level = ctx;
    let mut cleaned = path;
    while let Some(rest) = cleaned.strip_prefix("../") {
        cleaned = rest;
        if let Ctx::Level { parent, .. } = level {
            level = parent;
        }
    }
    let map = match level {
        Ctx::Level { map, .. } => *map,
        Ctx::Root => return None,
    };
    let mut parts = cleaned.split('/');
    let first = parts.next()?;
    let mut current = map.iter().find(|(k, _)| k == first).map(|(_, v)| v)?;
    for part in parts {
        current = match current {
            PValue::Compound(c) => c.iter().find(|(k, _)| k == part).map(|(_, v)| v)?,
            _ => return None,
        };
    }
    Some(current)
}

// ── Byte helpers ──

fn take<'a>(buf: &'a [u8], pos: usize, n: usize) -> Res<&'a [u8]> {
    buf.get(pos..pos + n)
        .ok_or_else(|| CodecError("unexpected end of buffer".into()))
}

// ── Read ──

pub fn read(reg: &TypeRegistry, schema: &Value, buf: &[u8], pos: usize) -> Res<(PValue, usize)> {
    read_ctx(reg, schema, buf, pos, &Ctx::Root)
}

fn read_ctx(
    reg: &TypeRegistry,
    schema: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    match schema {
        Value::String(name) => read_named(reg, name, buf, pos, ctx),
        Value::Array(arr) => {
            let type_name = arr
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| CodecError("schema array missing type name".into()))?;
            let params = arr.get(1).unwrap_or(&Value::Null);
            read_compound_type(reg, type_name, params, buf, pos, ctx)
        }
        _ => err(format!("invalid schema: {schema}")),
    }
}

fn read_named(
    reg: &TypeRegistry,
    name: &str,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    if let Some(r) = read_primitive(name, buf, pos)? {
        return Ok(r);
    }
    match reg.get(name) {
        Some(schema) => {
            let schema = schema.clone();
            read_ctx(reg, &schema, buf, pos, ctx)
        }
        None => err(format!("unknown type: {name}")),
    }
}

/// Returns `Some` for built-in primitive/custom types, `None` for named types.
fn read_primitive(name: &str, buf: &[u8], pos: usize) -> Res<Option<(PValue, usize)>> {
    let r = match name {
        "void" | "native" => (PValue::Void, 0),
        "bool" => (PValue::Bool(take(buf, pos, 1)?[0] != 0), 1),
        "i8" => (PValue::num(take(buf, pos, 1)?[0] as i8 as f64), 1),
        "u8" => (PValue::num(take(buf, pos, 1)?[0] as f64), 1),
        "i16" => (
            PValue::num(i16::from_be_bytes(take(buf, pos, 2)?.try_into().unwrap()) as f64),
            2,
        ),
        "u16" => (
            PValue::num(u16::from_be_bytes(take(buf, pos, 2)?.try_into().unwrap()) as f64),
            2,
        ),
        "i32" => (
            PValue::num(i32::from_be_bytes(take(buf, pos, 4)?.try_into().unwrap()) as f64),
            4,
        ),
        "u32" => (
            PValue::num(u32::from_be_bytes(take(buf, pos, 4)?.try_into().unwrap()) as f64),
            4,
        ),
        "i64" => (
            PValue::Long(i64::from_be_bytes(take(buf, pos, 8)?.try_into().unwrap())),
            8,
        ),
        "u64" => (
            PValue::ULong(u64::from_be_bytes(take(buf, pos, 8)?.try_into().unwrap())),
            8,
        ),
        "f32" => (
            PValue::num(f32::from_be_bytes(take(buf, pos, 4)?.try_into().unwrap()) as f64),
            4,
        ),
        "f64" => (
            PValue::num(f64::from_be_bytes(take(buf, pos, 8)?.try_into().unwrap())),
            8,
        ),
        "varint" => {
            let (v, s) = read_var_int(buf, pos).map_err(|e| CodecError(e.to_string()))?;
            (PValue::num(v as f64), s)
        }
        "varlong" => {
            let (v, s) = read_var_long(buf, pos).map_err(|e| CodecError(e.to_string()))?;
            (PValue::Long(v), s)
        }
        "UUID" => {
            let b = take(buf, pos, 16)?;
            let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
            let uuid = format!(
                "{}-{}-{}-{}-{}",
                &hex[0..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..32]
            );
            (PValue::Str(uuid), 16)
        }
        "restBuffer" => (PValue::Bytes(buf[pos..].to_vec()), buf.len() - pos),
        "anonymousNbt" => {
            let (v, s) = nbt::read_anonymous(buf, pos, NbtFormat::Big)
                .map_err(|e| CodecError(e.to_string()))?;
            (PValue::Nbt(v), s)
        }
        "anonOptionalNbt" => {
            if pos >= buf.len() || buf[pos] == 0 {
                (PValue::Nbt(None), 1)
            } else {
                let (v, s) = nbt::read_anonymous(buf, pos, NbtFormat::Big)
                    .map_err(|e| CodecError(e.to_string()))?;
                (PValue::Nbt(v), s)
            }
        }
        "lpVec3" => read_lp_vec3(buf, pos)?,
        _ => return Ok(None),
    };
    Ok(Some(r))
}

fn read_compound_type(
    reg: &TypeRegistry,
    type_name: &str,
    params: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    match type_name {
        "pstring" | "buffer" => {
            let (count, prefix) = read_count(reg, params, buf, pos, ctx)?;
            let data = take(buf, pos + prefix, count)?;
            let value = if type_name == "pstring" {
                PValue::Str(String::from_utf8_lossy(data).into_owned())
            } else {
                PValue::Bytes(data.to_vec())
            };
            Ok((value, prefix + count))
        }
        "container" => read_container(reg, params, buf, pos, ctx),
        "array" => read_array(reg, params, buf, pos, ctx),
        "mapper" => {
            let inner = &params["type"];
            let (v, s) = read_ctx(reg, inner, buf, pos, ctx)?;
            let key = v.to_key();
            let mapped = params["mappings"]
                .as_object()
                .and_then(|m| m.get(&map_key(&key)).or_else(|| m.get(&key)))
                .and_then(Value::as_str);
            Ok((mapped.map(PValue::str).unwrap_or(v), s))
        }
        "switch" => {
            let ty = switch_type(params, ctx);
            match ty {
                Some(t) => read_ctx(reg, &t, buf, pos, ctx),
                None => Ok((PValue::Void, 0)),
            }
        }
        "option" => {
            let present = take(buf, pos, 1)?[0] != 0;
            if !present {
                Ok((PValue::Void, 1))
            } else {
                let (v, s) = read_ctx(reg, params, buf, pos + 1, ctx)?;
                Ok((v, 1 + s))
            }
        }
        "bitfield" => read_bitfield(params, buf, pos),
        "bitflags" => read_bitflags(reg, params, buf, pos, ctx),
        "entityMetadataLoop" => read_entity_metadata_loop(reg, params, buf, pos, ctx),
        "topBitSetTerminatedArray" => read_top_bit_set(reg, params, buf, pos, ctx),
        "registryEntryHolder" => read_registry_entry_holder(reg, params, buf, pos, ctx),
        "registryEntryHolderSet" => read_registry_entry_holder_set(reg, params, buf, pos, ctx),
        _ => err(format!("unknown compound type: {type_name}")),
    }
}

fn read_count(
    reg: &TypeRegistry,
    params: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(usize, usize)> {
    if let Some(c) = params.get("count").and_then(Value::as_u64) {
        return Ok((c as usize, 0));
    }
    let ct = params.get("countType").unwrap_or(&Value::Null);
    let (v, s) = read_ctx(reg, ct, buf, pos, ctx)?;
    Ok((v.as_i64().unwrap_or(0).max(0) as usize, s))
}

fn read_container(
    reg: &TypeRegistry,
    fields: &Value,
    buf: &[u8],
    mut pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    let fields = fields
        .as_array()
        .ok_or_else(|| CodecError("container fields not array".into()))?;
    let start = pos;
    let mut map: Vec<(String, PValue)> = Vec::new();
    for field in fields {
        let anon = field.get("anon").and_then(Value::as_bool).unwrap_or(false);
        let name = field.get("name").and_then(Value::as_str);
        let ty = &field["type"];
        let (val, p) = {
            let child = Ctx::Level {
                map: &map,
                parent: ctx,
            };
            read_ctx(reg, ty, buf, pos, &child)?
        };
        pos += p;
        if anon {
            if let PValue::Compound(inner) = val {
                map.extend(inner);
            }
        } else if let Some(name) = name {
            map.push((name.to_string(), val));
        }
    }
    Ok((PValue::Compound(map), pos - start))
}

fn read_array(
    reg: &TypeRegistry,
    params: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    let elem = &params["type"];
    let mut total = 0usize;
    let count: usize = if let Some(n) = params.get("count").and_then(Value::as_u64) {
        n as usize
    } else if let Some(field) = params.get("count").and_then(Value::as_str) {
        resolve_compare_to(field, ctx)
            .and_then(PValue::as_i64)
            .unwrap_or(0)
            .max(0) as usize
    } else if let Some(ct) = params.get("countType") {
        let (v, s) = read_ctx(reg, ct, buf, pos, ctx)?;
        total += s;
        v.as_i64().unwrap_or(0).max(0) as usize
    } else {
        0
    };
    let mut list = Vec::with_capacity(count);
    for _ in 0..count {
        let (v, s) = read_ctx(reg, elem, buf, pos + total, ctx)?;
        list.push(v);
        total += s;
    }
    Ok((PValue::List(list), total))
}

fn read_bitfield(params: &Value, buf: &[u8], pos: usize) -> Res<(PValue, usize)> {
    let fields = params
        .as_array()
        .ok_or_else(|| CodecError("bitfield not array".into()))?;
    let total_bits: u32 = fields
        .iter()
        .map(|f| f["size"].as_u64().unwrap_or(0) as u32)
        .sum();
    let byte_size = total_bits.div_ceil(8) as usize;
    let bytes = take(buf, pos, byte_size)?;
    let mut raw: u128 = 0;
    for &b in bytes {
        raw = (raw << 8) | b as u128;
    }
    let mut map = Vec::new();
    let mut bit_offset = total_bits as i64;
    for f in fields {
        let size = f["size"].as_u64().unwrap_or(0) as u32;
        let signed = f["signed"].as_bool().unwrap_or(false);
        bit_offset -= size as i64;
        let mask: u128 = (1u128 << size) - 1;
        let mut val = ((raw >> bit_offset) & mask) as i128;
        if signed && val >= (1i128 << (size - 1)) {
            val -= 1i128 << size;
        }
        let name = f["name"].as_str().unwrap_or("").to_string();
        map.push((name, PValue::num(val as f64)));
    }
    Ok((PValue::Compound(map), byte_size))
}

fn read_bitflags(
    reg: &TypeRegistry,
    params: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    let (v, s) = read_ctx(reg, &params["type"], buf, pos, ctx)?;
    let bits = v.as_i64().unwrap_or(0);
    let shift = params.get("shift").and_then(Value::as_i64).unwrap_or(0);
    let flags = params["flags"].as_array().cloned().unwrap_or_default();
    let mut map = Vec::new();
    for (i, flag) in flags.iter().enumerate() {
        if let Some(name) = flag.as_str() {
            let set = (bits & (1 << (i as i64 + shift))) != 0;
            map.push((name.to_string(), PValue::Bool(set)));
        }
    }
    Ok((PValue::Compound(map), s))
}

fn read_entity_metadata_loop(
    reg: &TypeRegistry,
    params: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    let end_val = params["endVal"].as_u64().unwrap_or(0xff) as u8;
    let inner = &params["type"];
    let mut list = Vec::new();
    let mut total = 0usize;
    while pos + total < buf.len() {
        if buf[pos + total] == end_val {
            total += 1;
            break;
        }
        let (v, s) = read_ctx(reg, inner, buf, pos + total, ctx)?;
        if s == 0 {
            break;
        }
        list.push(v);
        total += s;
    }
    Ok((PValue::List(list), total))
}

fn read_top_bit_set(
    reg: &TypeRegistry,
    params: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    let inner = &params["type"];
    let mut list = Vec::new();
    let mut total = 0usize;
    while pos + total < buf.len() {
        let has_more = buf[pos + total] & 0x80 != 0;
        let mut tmp = buf[pos + total..].to_vec();
        tmp[0] &= 0x7f;
        let (v, s) = read_ctx(reg, inner, &tmp, 0, ctx)?;
        list.push(v);
        total += s;
        if !has_more {
            break;
        }
    }
    Ok((PValue::List(list), total))
}

fn read_registry_entry_holder(
    reg: &TypeRegistry,
    params: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    let base_name = params["baseName"].as_str().unwrap_or("").to_string();
    let other_name = params["otherwise"]["name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let other_ty = &params["otherwise"]["type"];
    let (id, s) = read_var_int(buf, pos).map_err(|e| CodecError(e.to_string()))?;
    if id == 0 {
        let (inner, is) = read_ctx(reg, other_ty, buf, pos + s, ctx)?;
        Ok((PValue::Compound(vec![(other_name, inner)]), s + is))
    } else {
        Ok((
            PValue::Compound(vec![(base_name, PValue::num((id - 1) as f64))]),
            s,
        ))
    }
}

fn read_registry_entry_holder_set(
    reg: &TypeRegistry,
    params: &Value,
    buf: &[u8],
    pos: usize,
    ctx: &Ctx,
) -> Res<(PValue, usize)> {
    let base_name = params["base"]["name"].as_str().unwrap_or("").to_string();
    let base_ty = &params["base"]["type"];
    let other_name = params["otherwise"]["name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let other_ty = &params["otherwise"]["type"];
    let (disc, s) = read_var_int(buf, pos).map_err(|e| CodecError(e.to_string()))?;
    if disc == 0 {
        let (inner, is) = read_ctx(reg, base_ty, buf, pos + s, ctx)?;
        Ok((PValue::Compound(vec![(base_name, inner)]), s + is))
    } else {
        let count = (disc - 1) as usize;
        let mut total = s;
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let (v, es) = read_ctx(reg, other_ty, buf, pos + total, ctx)?;
            ids.push(v);
            total += es;
        }
        Ok((
            PValue::Compound(vec![(other_name, PValue::List(ids))]),
            total,
        ))
    }
}

// ── lpVec3 ──

const LP_VEC3_DATA_BITS_MASK: i64 = 32767;
const LP_VEC3_MAX_QUANTIZED: f64 = 32766.0;
const LP_VEC3_ABS_MIN: f64 = 3.051944088384301e-5;
const LP_VEC3_ABS_MAX: f64 = 1.7179869183e10;

fn lp_unpack(packed: i64, shift: u32) -> f64 {
    let val = (packed >> shift) & LP_VEC3_DATA_BITS_MASK;
    let clamped = if val > 32766 { 32766 } else { val };
    (clamped as f64 * 2.0) / 32766.0 - 1.0
}

fn lp_pack(value: f64) -> i64 {
    ((value * 0.5 + 0.5) * LP_VEC3_MAX_QUANTIZED).round() as i64
}

fn lp_sanitize(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-LP_VEC3_ABS_MAX, LP_VEC3_ABS_MAX)
    }
}

fn read_lp_vec3(buf: &[u8], pos: usize) -> Res<(PValue, usize)> {
    let a = take(buf, pos, 1)?[0] as i64;
    if a == 0 {
        return Ok((
            PValue::compound(vec![
                ("x", PValue::num(0.0)),
                ("y", PValue::num(0.0)),
                ("z", PValue::num(0.0)),
            ]),
            1,
        ));
    }
    let byte1 = take(buf, pos + 1, 1)?[0] as i64;
    let dword = u32::from_le_bytes(take(buf, pos + 2, 4)?.try_into().unwrap()) as i64;
    let packed = dword * 65536 + (byte1 << 8) + a;
    let mut scale = a & 3;
    let mut size = 6;
    if a & 4 == 4 {
        let (v, s) = read_var_int(buf, pos + 6).map_err(|e| CodecError(e.to_string()))?;
        scale = v as i64 * 4 + scale;
        size += s;
    }
    Ok((
        PValue::compound(vec![
            ("x", PValue::num(lp_unpack(packed, 3) * scale as f64)),
            ("y", PValue::num(lp_unpack(packed, 18) * scale as f64)),
            ("z", PValue::num(lp_unpack(packed, 33) * scale as f64)),
        ]),
        size,
    ))
}

fn write_lp_vec3(v: &PValue, out: &mut Vec<u8>) {
    let x = lp_sanitize(v.get("x").and_then(PValue::as_f64).unwrap_or(0.0));
    let y = lp_sanitize(v.get("y").and_then(PValue::as_f64).unwrap_or(0.0));
    let z = lp_sanitize(v.get("z").and_then(PValue::as_f64).unwrap_or(0.0));
    let max = x.abs().max(y.abs()).max(z.abs());
    if max < LP_VEC3_ABS_MIN {
        out.push(0);
        return;
    }
    let scale = max.ceil() as i64;
    let needs_cont = (scale & 3) != scale;
    let scale_byte = if needs_cont {
        (scale & 3) | 4
    } else {
        scale & 3
    };
    let p_x = lp_pack(x / scale as f64);
    let p_y = lp_pack(y / scale as f64);
    let p_z = lp_pack(z / scale as f64);
    let low32 = (scale_byte | (p_x << 3) | (p_y << 18)) as u32;
    let high16 = (((p_y >> 14) & 0x01) | (p_z << 1)) as u16;
    out.extend_from_slice(&low32.to_le_bytes());
    out.extend_from_slice(&high16.to_le_bytes());
    if needs_cont {
        push_var_int(out, (scale / 4) as i32);
    }
}

// ── switch / mapper helpers ──

fn map_key(key: &str) -> String {
    // mapper mappings may be keyed by "0x.." or decimal; the value came in as a
    // number, so try the hex form too.
    if let Ok(n) = key.parse::<i64>() {
        format!("0x{n:02x}")
    } else {
        key.to_string()
    }
}

fn switch_type(params: &Value, ctx: &Ctx) -> Option<Value> {
    let compare_to = params["compareTo"].as_str()?;
    let val = resolve_compare_to(compare_to, ctx);
    let key = val
        .map(PValue::to_key)
        .unwrap_or_else(|| "undefined".to_string());
    if let Some(t) = params["fields"].get(&key) {
        return Some(t.clone());
    }
    params.get("default").cloned()
}

// ── Write ──

pub fn write(reg: &TypeRegistry, schema: &Value, value: &PValue, out: &mut Vec<u8>) -> Res<()> {
    write_ctx(reg, schema, value, out, &Ctx::Root)
}

/// Write with a seeded parent context (e.g. component serialization seeds
/// `{ type }` so inner `compareTo: "../type"` switches resolve).
pub(crate) fn write_seeded(
    reg: &TypeRegistry,
    schema: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    seed: &[(String, PValue)],
) -> Res<()> {
    let root = Ctx::Root;
    let level = Ctx::Level {
        map: seed,
        parent: &root,
    };
    write_ctx(reg, schema, value, out, &level)
}

fn write_ctx(
    reg: &TypeRegistry,
    schema: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    match schema {
        Value::String(name) => write_named(reg, name, value, out, ctx),
        Value::Array(arr) => {
            let type_name = arr
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| CodecError("schema array missing type name".into()))?;
            let params = arr.get(1).unwrap_or(&Value::Null);
            write_compound_type(reg, type_name, params, value, out, ctx)
        }
        _ => err(format!("invalid schema: {schema}")),
    }
}

fn write_named(
    reg: &TypeRegistry,
    name: &str,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    if write_primitive(name, value, out)? {
        return Ok(());
    }
    match reg.get(name) {
        Some(schema) => {
            let schema = schema.clone();
            write_ctx(reg, &schema, value, out, ctx)
        }
        None => err(format!("unknown type: {name}")),
    }
}

fn write_primitive(name: &str, value: &PValue, out: &mut Vec<u8>) -> Res<bool> {
    match name {
        "void" | "native" => {}
        "bool" => out.push(value.as_bool().unwrap_or(false) as u8),
        "i8" => out.push(value.as_i64().unwrap_or(0) as i8 as u8),
        "u8" => out.push(value.as_i64().unwrap_or(0) as u8),
        "i16" => out.extend_from_slice(&(value.as_i64().unwrap_or(0) as i16).to_be_bytes()),
        "u16" => out.extend_from_slice(&(value.as_i64().unwrap_or(0) as u16).to_be_bytes()),
        "i32" => out.extend_from_slice(&(value.as_i64().unwrap_or(0) as i32).to_be_bytes()),
        "u32" => out.extend_from_slice(&(value.as_i64().unwrap_or(0) as u32).to_be_bytes()),
        "i64" => out.extend_from_slice(&value.as_i64().unwrap_or(0).to_be_bytes()),
        "u64" => {
            let u = match value {
                PValue::ULong(u) => *u,
                other => other.as_i64().unwrap_or(0) as u64,
            };
            out.extend_from_slice(&u.to_be_bytes());
        }
        "f32" => out.extend_from_slice(&(value.as_f64().unwrap_or(0.0) as f32).to_be_bytes()),
        "f64" => out.extend_from_slice(&value.as_f64().unwrap_or(0.0).to_be_bytes()),
        "varint" => push_var_int(out, value.as_i64().unwrap_or(0) as i32),
        "varlong" => push_var_long(out, value.as_i64().unwrap_or(0)),
        "UUID" => {
            let s = value.as_str().unwrap_or("");
            let hex: String = s.chars().filter(|c| *c != '-').collect();
            for i in 0..16 {
                let byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
                out.push(byte);
            }
        }
        "restBuffer" => out.extend_from_slice(value.as_bytes().unwrap_or(&[])),
        "anonymousNbt" => {
            let buf = nbt::write_anonymous(value.as_nbt(), NbtFormat::Big);
            out.extend_from_slice(&buf);
        }
        "anonOptionalNbt" => {
            let buf = nbt::write_anonymous(value.as_nbt(), NbtFormat::Big);
            out.extend_from_slice(&buf);
        }
        "lpVec3" => write_lp_vec3(value, out),
        _ => return Ok(false),
    }
    Ok(true)
}

fn write_compound_type(
    reg: &TypeRegistry,
    type_name: &str,
    params: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    match type_name {
        "pstring" => {
            let s = value.as_str().unwrap_or("");
            let bytes = s.as_bytes();
            write_count(reg, params, bytes.len(), out, ctx)?;
            out.extend_from_slice(bytes);
            Ok(())
        }
        "buffer" => {
            let bytes = value.as_bytes().unwrap_or(&[]);
            if params.get("count").and_then(Value::as_u64).is_none() {
                write_count(reg, params, bytes.len(), out, ctx)?;
            }
            out.extend_from_slice(bytes);
            Ok(())
        }
        "container" => write_container(reg, params, value, out, ctx),
        "array" => write_array(reg, params, value, out, ctx),
        "mapper" => {
            let inner = &params["type"];
            let key = value.to_key();
            let mapped = params["mappings"].as_object().and_then(|m| {
                m.iter()
                    .find(|(_, v)| v.as_str() == Some(&key))
                    .map(|(k, _)| k.clone())
            });
            let num = mapped
                .map(|k| parse_int_key(&k))
                .or_else(|| value.as_i64())
                .unwrap_or(0);
            write_ctx(reg, inner, &PValue::num(num as f64), out, ctx)
        }
        "switch" => {
            if let Some(t) = switch_type(params, ctx) {
                write_ctx(reg, &t, value, out, ctx)
            } else {
                Ok(())
            }
        }
        "option" => {
            let absent = matches!(value, PValue::Void) || matches!(value, PValue::Bool(false));
            if absent {
                out.push(0);
                Ok(())
            } else {
                out.push(1);
                write_ctx(reg, params, value, out, ctx)
            }
        }
        "bitfield" => write_bitfield(params, value, out),
        "bitflags" => write_bitflags(reg, params, value, out, ctx),
        "entityMetadataLoop" => {
            let inner = &params["type"];
            let end_val = params["endVal"].as_u64().unwrap_or(0xff) as u8;
            for elem in value.as_list().unwrap_or(&[]) {
                write_ctx(reg, inner, elem, out, ctx)?;
            }
            out.push(end_val);
            Ok(())
        }
        "topBitSetTerminatedArray" => write_top_bit_set(reg, params, value, out, ctx),
        "registryEntryHolder" => write_registry_entry_holder(reg, params, value, out, ctx),
        "registryEntryHolderSet" => write_registry_entry_holder_set(reg, params, value, out, ctx),
        _ => err(format!("unknown compound type: {type_name}")),
    }
}

fn write_count(
    reg: &TypeRegistry,
    params: &Value,
    count: usize,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    let ct = params.get("countType").unwrap_or(&Value::Null);
    write_ctx(reg, ct, &PValue::num(count as f64), out, ctx)
}

fn write_container(
    reg: &TypeRegistry,
    fields: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    let fields = fields
        .as_array()
        .ok_or_else(|| CodecError("container fields not array".into()))?;
    let map = value.as_compound().unwrap_or(&[]);
    for field in fields {
        let anon = field.get("anon").and_then(Value::as_bool).unwrap_or(false);
        let name = field.get("name").and_then(Value::as_str);
        let ty = &field["type"];
        let child = Ctx::Level { map, parent: ctx };
        if anon {
            write_ctx(reg, ty, value, out, &child)?;
        } else if let Some(name) = name {
            let v = map.iter().find(|(k, _)| k == name).map(|(_, v)| v);
            let v = v.cloned().unwrap_or(PValue::Void);
            write_ctx(reg, ty, &v, out, &child)?;
        }
    }
    Ok(())
}

fn write_array(
    reg: &TypeRegistry,
    params: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    let elem = &params["type"];
    let list = value.as_list().unwrap_or(&[]);
    if params.get("countType").is_some() {
        write_count(reg, params, list.len(), out, ctx)?;
    }
    for v in list {
        write_ctx(reg, elem, v, out, ctx)?;
    }
    Ok(())
}

fn write_bitfield(params: &Value, value: &PValue, out: &mut Vec<u8>) -> Res<()> {
    let fields = params
        .as_array()
        .ok_or_else(|| CodecError("bitfield not array".into()))?;
    let total_bits: u32 = fields
        .iter()
        .map(|f| f["size"].as_u64().unwrap_or(0) as u32)
        .sum();
    let byte_size = total_bits.div_ceil(8) as usize;
    let mut raw: u128 = 0;
    let mut bit_offset = total_bits as i64;
    for f in fields {
        let size = f["size"].as_u64().unwrap_or(0) as u32;
        let name = f["name"].as_str().unwrap_or("");
        bit_offset -= size as i64;
        let mask: u128 = (1u128 << size) - 1;
        let mut val = value.get(name).and_then(PValue::as_i64).unwrap_or(0) as i128;
        if val < 0 {
            val += 1i128 << size;
        }
        raw |= ((val as u128) & mask) << bit_offset;
    }
    for i in 0..byte_size {
        let shift = (byte_size - 1 - i) * 8;
        out.push(((raw >> shift) & 0xff) as u8);
    }
    Ok(())
}

fn write_bitflags(
    reg: &TypeRegistry,
    params: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    let shift = params.get("shift").and_then(Value::as_i64).unwrap_or(0);
    let flags = params["flags"].as_array().cloned().unwrap_or_default();
    let mut bits: i64 = 0;
    for (i, flag) in flags.iter().enumerate() {
        if let Some(name) = flag.as_str() {
            if value.get(name).and_then(PValue::as_bool).unwrap_or(false) {
                bits |= 1 << (i as i64 + shift);
            }
        }
    }
    write_ctx(reg, &params["type"], &PValue::num(bits as f64), out, ctx)
}

fn write_top_bit_set(
    reg: &TypeRegistry,
    params: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    let inner = &params["type"];
    let list = value.as_list().unwrap_or(&[]);
    for (i, elem) in list.iter().enumerate() {
        let start = out.len();
        write_ctx(reg, inner, elem, out, ctx)?;
        if i < list.len() - 1 {
            out[start] |= 0x80;
        }
    }
    Ok(())
}

fn write_registry_entry_holder(
    reg: &TypeRegistry,
    params: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    let base_name = params["baseName"].as_str().unwrap_or("");
    let other_name = params["otherwise"]["name"].as_str().unwrap_or("");
    let other_ty = &params["otherwise"]["type"];
    if let Some(id) = value.get(base_name).and_then(PValue::as_i64) {
        push_var_int(out, (id + 1) as i32);
        Ok(())
    } else {
        push_var_int(out, 0);
        let v = value.get(other_name).cloned().unwrap_or(PValue::Void);
        write_ctx(reg, other_ty, &v, out, ctx)
    }
}

fn write_registry_entry_holder_set(
    reg: &TypeRegistry,
    params: &Value,
    value: &PValue,
    out: &mut Vec<u8>,
    ctx: &Ctx,
) -> Res<()> {
    let base_name = params["base"]["name"].as_str().unwrap_or("");
    let base_ty = &params["base"]["type"];
    let other_name = params["otherwise"]["name"].as_str().unwrap_or("");
    let other_ty = &params["otherwise"]["type"];
    if let Some(v) = value.get(base_name) {
        push_var_int(out, 0);
        let v = v.clone();
        write_ctx(reg, base_ty, &v, out, ctx)
    } else {
        let ids = value
            .get(other_name)
            .and_then(PValue::as_list)
            .unwrap_or(&[])
            .to_vec();
        push_var_int(out, (ids.len() + 1) as i32);
        for id in &ids {
            write_ctx(reg, other_ty, id, out, ctx)?;
        }
        Ok(())
    }
}

fn parse_int_key(k: &str) -> i64 {
    if let Some(hex) = k.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).unwrap_or(0)
    } else {
        k.parse().unwrap_or(0)
    }
}

// ── Packet-level codec ──

pub struct PacketCodec {
    registry: TypeRegistry,
    pub packet_names: HashMap<i32, String>,
    pub packet_ids: HashMap<String, i32>,
    packet_types: HashMap<String, Value>,
}

impl PacketCodec {
    /// Build a codec from a merged types map (shared types ∪ state types).
    pub fn new(types: HashMap<String, Value>) -> Res<PacketCodec> {
        let packet = types
            .get("packet")
            .and_then(Value::as_array)
            .ok_or_else(|| CodecError("no packet type".into()))?;
        let fields = packet
            .get(1)
            .and_then(Value::as_array)
            .ok_or_else(|| CodecError("packet container has no fields".into()))?;

        let name_field = fields
            .iter()
            .find(|f| f["name"] == Value::String("name".into()))
            .ok_or_else(|| CodecError("packet has no name field".into()))?;
        let mappings = name_field["type"][1]["mappings"]
            .as_object()
            .ok_or_else(|| CodecError("name mapper has no mappings".into()))?;

        let mut packet_names = HashMap::new();
        let mut packet_ids = HashMap::new();
        for (id_str, name) in mappings {
            let id = parse_int_key(id_str) as i32;
            let name = name.as_str().unwrap_or("").to_string();
            packet_names.insert(id, name.clone());
            packet_ids.insert(name, id);
        }

        let params_field = fields
            .iter()
            .find(|f| f["name"] == Value::String("params".into()))
            .ok_or_else(|| CodecError("packet has no params field".into()))?;
        let switch_fields = params_field["type"][1]["fields"]
            .as_object()
            .ok_or_else(|| CodecError("params switch has no fields".into()))?;

        let mut packet_types = HashMap::new();
        for (name, type_ref) in switch_fields {
            packet_types.insert(name.clone(), type_ref.clone());
        }

        Ok(PacketCodec {
            registry: TypeRegistry::new(types),
            packet_names,
            packet_ids,
            packet_types,
        })
    }

    pub fn read(&self, buffer: &[u8]) -> Res<(String, PValue)> {
        let (id, id_size) = read_var_int(buffer, 0).map_err(|e| CodecError(e.to_string()))?;
        let name = self
            .packet_names
            .get(&id)
            .ok_or_else(|| CodecError(format!("unknown packet id: 0x{id:x}")))?
            .clone();
        let schema = self
            .packet_types
            .get(&name)
            .ok_or_else(|| CodecError(format!("no type for packet: {name}")))?
            .clone();
        let (params, _) = read(&self.registry, &schema, buffer, id_size)?;
        Ok((name, params))
    }

    pub fn write(&self, name: &str, params: &PValue) -> Res<Vec<u8>> {
        let id = *self
            .packet_ids
            .get(name)
            .ok_or_else(|| CodecError(format!("unknown packet name: {name}")))?;
        let schema = self
            .packet_types
            .get(name)
            .ok_or_else(|| CodecError(format!("no type for packet: {name}")))?
            .clone();
        let mut out = Vec::new();
        push_var_int(&mut out, id);
        write(&self.registry, &schema, params, &mut out)?;
        Ok(out)
    }
}
