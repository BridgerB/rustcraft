//! Dynamic protocol value — the runtime representation of any packet field,
//! analogous to the `unknown` values flowing through typecraft's codec.

use crate::nbt::NbtRoot;

#[derive(Debug, Clone, PartialEq)]
pub enum PValue {
    /// Absent / void (also used for an absent `option`).
    Void,
    Bool(bool),
    /// Any number-typed field (i8..u32, f32, f64, varint). Mirrors JS `number`.
    Num(f64),
    /// 64-bit signed (i64 / varlong).
    Long(i64),
    /// 64-bit unsigned (u64).
    ULong(u64),
    Str(String),
    Bytes(Vec<u8>),
    Nbt(Option<NbtRoot>),
    List(Vec<PValue>),
    Compound(Vec<(String, PValue)>),
}

impl PValue {
    pub fn num(n: impl Into<f64>) -> PValue {
        PValue::Num(n.into())
    }

    pub fn str(s: impl Into<String>) -> PValue {
        PValue::Str(s.into())
    }

    pub fn compound(entries: Vec<(&str, PValue)>) -> PValue {
        PValue::Compound(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    pub fn is_void(&self) -> bool {
        matches!(self, PValue::Void)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            PValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PValue::Num(n) => Some(*n),
            PValue::Long(l) => Some(*l as f64),
            PValue::ULong(u) => Some(*u as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            PValue::Num(n) => Some(*n as i64),
            PValue::Long(l) => Some(*l),
            PValue::ULong(u) => Some(*u as i64),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        self.as_i64().map(|v| v as i32)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            PValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            PValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[PValue]> {
        match self {
            PValue::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_compound(&self) -> Option<&[(String, PValue)]> {
        match self {
            PValue::Compound(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_nbt(&self) -> Option<&NbtRoot> {
        match self {
            PValue::Nbt(Some(n)) => Some(n),
            _ => None,
        }
    }

    /// Look up a key in a compound value.
    pub fn get(&self, key: &str) -> Option<&PValue> {
        match self {
            PValue::Compound(c) => c.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Stringify for `switch`/`compareTo` keying, matching JS `String(val)`.
    pub fn to_key(&self) -> String {
        match self {
            PValue::Num(n) => {
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            PValue::Long(l) => l.to_string(),
            PValue::ULong(u) => u.to_string(),
            PValue::Bool(b) => b.to_string(),
            PValue::Str(s) => s.clone(),
            PValue::Void => "undefined".to_string(),
            _ => String::new(),
        }
    }
}
