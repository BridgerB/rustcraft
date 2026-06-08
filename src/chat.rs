//! Parse and render Minecraft chat messages (JSON / NBT components → text,
//! MOTD, ANSI). Port of typecraft's `chat` module (text-rendering subset; HTML
//! and the builder are omitted).

use std::collections::HashMap;

use serde_json::Value;

use crate::nbt::{NbtRoot, Simple};
use crate::protocol::PValue;
use crate::registry::Registry;

const MAX_CHAT_DEPTH: u32 = 8;
const MAX_CHAT_LENGTH: usize = 4096;

/// Named language map: translate-key → format string.
pub type Language = HashMap<String, String>;

/// A parsed chat message tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatMessage {
    pub text: Option<String>,
    pub translate: Option<String>,
    pub fallback: Option<String>,
    pub with: Vec<ChatMessage>,
    pub extra: Vec<ChatMessage>,
    pub color: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub strikethrough: bool,
    pub obfuscated: bool,
    pub reset: bool,
}

// ── vsprintf ──

/// Printf-style format: `%s` (sequential) and `%N$s` (positional), `%%` → `%`.
pub fn vsprintf(format: &str, args: &[String]) -> String {
    let bytes = format.as_bytes();
    let mut out = String::new();
    let mut seq = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        // parse %(\d+\$)?(s|%)
        let mut j = i + 1;
        let mut num = String::new();
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            num.push(bytes[j] as char);
            j += 1;
        }
        let mut positional = None;
        if !num.is_empty() && j < bytes.len() && bytes[j] == b'$' {
            positional = num.parse::<usize>().ok();
            j += 1;
        } else {
            j = i + 1; // not a positional spec; reset
        }
        if j < bytes.len() && bytes[j] == b'%' && positional.is_none() {
            out.push('%');
            i = j + 1;
        } else if j < bytes.len() && bytes[j] == b's' {
            let idx = match positional {
                Some(n) => n - 1,
                None => {
                    let n = seq;
                    seq += 1;
                    n
                }
            };
            out.push_str(args.get(idx).map(String::as_str).unwrap_or(""));
            i = j + 1;
        } else {
            out.push('%');
            i += 1;
        }
    }
    out
}

fn strip_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

// ── Styles ──

fn motd_color_code(name: &str) -> &'static str {
    match name {
        "black" => "§0",
        "dark_blue" => "§1",
        "dark_green" => "§2",
        "dark_aqua" => "§3",
        "dark_red" => "§4",
        "dark_purple" => "§5",
        "gold" => "§6",
        "gray" => "§7",
        "dark_gray" => "§8",
        "blue" => "§9",
        "green" => "§a",
        "aqua" => "§b",
        "red" => "§c",
        "light_purple" => "§d",
        "yellow" => "§e",
        "white" => "§f",
        "reset" => "§r",
        _ => "",
    }
}

fn ansi_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "§0" => "\u{1b}[30m",
        "§1" => "\u{1b}[34m",
        "§2" => "\u{1b}[32m",
        "§3" => "\u{1b}[36m",
        "§4" => "\u{1b}[31m",
        "§5" => "\u{1b}[35m",
        "§6" => "\u{1b}[33m",
        "§7" => "\u{1b}[37m",
        "§8" => "\u{1b}[90m",
        "§9" => "\u{1b}[94m",
        "§a" => "\u{1b}[92m",
        "§b" => "\u{1b}[96m",
        "§c" => "\u{1b}[91m",
        "§d" => "\u{1b}[95m",
        "§e" => "\u{1b}[93m",
        "§f" => "\u{1b}[97m",
        "§l" => "\u{1b}[1m",
        "§o" => "\u{1b}[3m",
        "§n" => "\u{1b}[4m",
        "§m" => "\u{1b}[9m",
        "§k" => "\u{1b}[6m",
        "§r" => "\u{1b}[0m",
        _ => return None,
    })
}

const SUPPORTED_COLORS: &[&str] = &[
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
    "obfuscated",
    "bold",
    "strikethrough",
    "underlined",
    "italic",
    "reset",
];

// ── Parsing ──

/// Parse a chat message from a JSON value (string, number, array, or object).
pub fn parse_chat_message(value: &Value) -> ChatMessage {
    match value {
        Value::String(s) => ChatMessage {
            text: Some(s.clone()),
            ..Default::default()
        },
        Value::Number(n) => ChatMessage {
            text: Some(n.to_string()),
            ..Default::default()
        },
        Value::Array(arr) => ChatMessage {
            extra: arr.iter().map(parse_chat_message).collect(),
            ..Default::default()
        },
        Value::Object(_) => parse_object(value),
        _ => ChatMessage::default(),
    }
}

fn as_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn parse_object(json: &Value) -> ChatMessage {
    let mut msg = ChatMessage::default();

    if let Some(t) = json.get("text").and_then(as_text) {
        msg.text = Some(t);
    } else if let Some(t) = json.get("").and_then(as_text) {
        msg.text = Some(t);
    } else if let Some(translate) = json.get("translate").and_then(|v| v.as_str()) {
        msg.translate = Some(translate.to_string());
        msg.fallback = json
            .get("fallback")
            .and_then(|v| v.as_str())
            .map(String::from);
        if let Some(with) = json.get("with").and_then(|v| v.as_array()) {
            msg.with = with.iter().map(parse_chat_message).collect();
        }
    }

    if let Some(extra) = json.get("extra").and_then(|v| v.as_array()) {
        msg.extra = extra.iter().map(parse_chat_message).collect();
    }

    msg.bold = json.get("bold").and_then(|v| v.as_bool()).unwrap_or(false);
    msg.italic = json
        .get("italic")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    msg.underlined = json
        .get("underlined")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    msg.strikethrough = json
        .get("strikethrough")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    msg.obfuscated = json
        .get("obfuscated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let color = json.get("color").and_then(|v| v.as_str()).map(String::from);
    match color.as_deref() {
        Some("obfuscated") => msg.obfuscated = true,
        Some("bold") => msg.bold = true,
        Some("strikethrough") => msg.strikethrough = true,
        Some("underlined") => msg.underlined = true,
        Some("italic") => msg.italic = true,
        Some("reset") => msg.reset = true,
        Some(c) => {
            let is_hex = c.starts_with('#')
                && c.len() == 7
                && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
            if SUPPORTED_COLORS.contains(&c) || is_hex {
                msg.color = Some(c.to_string());
            }
        }
        None => {}
    }

    msg
}

// ── NBT message processing (1.20.3+) ──

fn uuid_from_int_array(arr: &[i32]) -> String {
    let mut bytes = Vec::with_capacity(16);
    for &v in arr {
        bytes.extend_from_slice(&v.to_be_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn simple_to_value(simple: &Simple) -> Value {
    match simple {
        Simple::Byte(v) => Value::from(*v),
        Simple::Short(v) => Value::from(*v),
        Simple::Int(v) => Value::from(*v),
        Simple::Long(v) => Value::from(*v),
        Simple::Float(v) => Value::from(*v),
        Simple::Double(v) => Value::from(*v),
        Simple::String(v) => Value::String(v.clone()),
        Simple::ByteArray(v) => Value::Array(v.iter().map(|&b| Value::from(b)).collect()),
        Simple::IntArray(v) => Value::Array(v.iter().map(|&n| Value::from(n)).collect()),
        Simple::LongArray(v) => Value::Array(v.iter().map(|&n| Value::from(n)).collect()),
        Simple::List(items) => Value::Array(items.iter().map(simple_to_value).collect()),
        Simple::Compound(entries) => {
            let mut map = serde_json::Map::new();
            for (k, v) in entries {
                // "id" int-arrays are UUIDs — render as a hex string.
                let value = if k == "id" {
                    if let Simple::IntArray(arr) = v {
                        Value::String(uuid_from_int_array(arr))
                    } else {
                        simple_to_value(v)
                    }
                } else {
                    simple_to_value(v)
                };
                map.insert(k.clone(), value);
            }
            Value::Object(map)
        }
    }
}

/// Normalize an NBT chat component to a JSON value.
pub fn process_nbt_message(root: &NbtRoot) -> Value {
    simple_to_value(&crate::nbt::simplify(&crate::nbt::NbtTag::Compound(
        root.value.clone(),
    )))
}

/// Parse a chat message from network packet content.
pub fn chat_from_notch(registry: &Registry, msg: &PValue) -> ChatMessage {
    if registry
        .support_feature("chatPacketsUseNbtComponents")
        .as_bool()
    {
        if let PValue::Nbt(Some(root)) = msg {
            return parse_chat_message(&process_nbt_message(root));
        }
    }
    match msg {
        PValue::Str(s) => match serde_json::from_str::<Value>(s) {
            Ok(v) => parse_chat_message(&v),
            Err(_) => ChatMessage {
                text: Some(s.clone()),
                ..Default::default()
            },
        },
        PValue::Nbt(Some(root)) => parse_chat_message(&process_nbt_message(root)),
        _ => ChatMessage::default(),
    }
}

// ── Rendering ──

/// Flatten to plain text (formatting codes stripped).
pub fn chat_to_string(msg: &ChatMessage, lang: &Language) -> String {
    let s = to_string_inner(msg, lang, 0);
    let stripped = strip_codes(&s);
    stripped.chars().take(MAX_CHAT_LENGTH).collect()
}

fn to_string_inner(msg: &ChatMessage, lang: &Language, depth: u32) -> String {
    if depth > MAX_CHAT_DEPTH {
        return String::new();
    }
    let mut message = String::new();
    if let Some(text) = &msg.text {
        message.push_str(text);
    } else if let Some(translate) = &msg.translate {
        let args: Vec<String> = msg
            .with
            .iter()
            .map(|e| to_string_inner(e, lang, depth + 1))
            .collect();
        let format = lang
            .get(translate)
            .cloned()
            .or_else(|| msg.fallback.clone())
            .unwrap_or_else(|| translate.clone());
        message.push_str(&vsprintf(&format, &args));
    }
    for entry in &msg.extra {
        message.push_str(&to_string_inner(entry, lang, depth + 1));
    }
    message
}

/// Flatten to MOTD format (`§` codes).
pub fn chat_to_motd(msg: &ChatMessage, lang: &Language) -> String {
    to_motd_inner(msg, lang, &Inherited::default(), 0)
}

#[derive(Default, Clone)]
struct Inherited {
    color: Option<String>,
    bold: bool,
    italic: bool,
    underlined: bool,
    strikethrough: bool,
    obfuscated: bool,
}

fn to_motd_inner(msg: &ChatMessage, lang: &Language, parent: &Inherited, depth: u32) -> String {
    if depth > MAX_CHAT_DEPTH {
        return String::new();
    }
    let color = msg.color.clone().or_else(|| parent.color.clone());
    let bold = msg.bold || parent.bold;
    let italic = msg.italic || parent.italic;
    let underlined = msg.underlined || parent.underlined;
    let strikethrough = msg.strikethrough || parent.strikethrough;
    let obfuscated = msg.obfuscated || parent.obfuscated;

    let mut prefix = String::new();
    if let Some(c) = &color {
        if let Some(stripped) = c.strip_prefix('#') {
            prefix.push_str(&format!("§#{stripped}"));
        } else {
            prefix.push_str(motd_color_code(c));
        }
    }
    if bold {
        prefix.push_str("§l");
    }
    if italic {
        prefix.push_str("§o");
    }
    if underlined {
        prefix.push_str("§n");
    }
    if strikethrough {
        prefix.push_str("§m");
    }
    if obfuscated {
        prefix.push_str("§k");
    }

    let inherited = Inherited {
        color,
        bold,
        italic,
        underlined,
        strikethrough,
        obfuscated,
    };

    let mut message = prefix;
    if let Some(text) = &msg.text {
        message.push_str(text);
    } else if let Some(translate) = &msg.translate {
        let args: Vec<String> = msg
            .with
            .iter()
            .map(|e| to_motd_inner(e, lang, &inherited, depth + 1))
            .collect();
        let format = lang
            .get(translate)
            .cloned()
            .or_else(|| msg.fallback.clone())
            .unwrap_or_else(|| translate.clone());
        message.push_str(&vsprintf(&format, &args));
    }
    for entry in &msg.extra {
        message.push_str(&to_motd_inner(entry, lang, &inherited, depth + 1));
    }
    message.chars().take(MAX_CHAT_LENGTH).collect()
}

/// Flatten to ANSI-colored terminal text.
pub fn chat_to_ansi(msg: &ChatMessage, lang: &Language) -> String {
    let mut message = chat_to_motd(msg, lang);
    for code in [
        "§0", "§1", "§2", "§3", "§4", "§5", "§6", "§7", "§8", "§9", "§a", "§b", "§c", "§d", "§e",
        "§f", "§l", "§o", "§n", "§m", "§k", "§r",
    ] {
        if let Some(ansi) = ansi_code(code) {
            message = message.replace(code, ansi);
        }
    }
    format!(
        "\u{1b}[0m{}\u{1b}[0m",
        message.chars().take(MAX_CHAT_LENGTH).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vsprintf_sequential_and_positional() {
        assert_eq!(vsprintf("Hello %s!", &["World".into()]), "Hello World!");
        assert_eq!(vsprintf("%s and %s", &["a".into(), "b".into()]), "a and b");
        assert_eq!(vsprintf("%2$s %1$s", &["a".into(), "b".into()]), "b a");
        assert_eq!(vsprintf("100%%", &[]), "100%");
    }

    #[test]
    fn parses_plain_string() {
        let msg = parse_chat_message(&json!("hello"));
        assert_eq!(chat_to_string(&msg, &Language::new()), "hello");
    }

    #[test]
    fn parses_nested_extra() {
        let msg = parse_chat_message(&json!({
            "text": "a",
            "extra": [{"text": "b"}, {"text": "c", "color": "red"}]
        }));
        assert_eq!(chat_to_string(&msg, &Language::new()), "abc");
    }

    #[test]
    fn translates_with_args() {
        let mut lang = Language::new();
        lang.insert("chat.type.text".into(), "<%s> %s".into());
        let msg = parse_chat_message(&json!({
            "translate": "chat.type.text",
            "with": ["Steve", "hello world"]
        }));
        assert_eq!(chat_to_string(&msg, &lang), "<Steve> hello world");
    }

    #[test]
    fn translate_falls_back_to_key() {
        let msg = parse_chat_message(&json!({"translate": "unknown.key", "with": []}));
        assert_eq!(chat_to_string(&msg, &Language::new()), "unknown.key");
    }

    #[test]
    fn color_promotes_format_names() {
        let msg = parse_chat_message(&json!({"text": "x", "color": "bold"}));
        assert!(msg.bold);
        assert!(msg.color.is_none());
    }

    #[test]
    fn motd_emits_color_codes() {
        let msg = parse_chat_message(&json!({"text": "hi", "color": "red", "bold": true}));
        let motd = chat_to_motd(&msg, &Language::new());
        assert!(motd.contains("§c"));
        assert!(motd.contains("§l"));
        assert!(motd.contains("hi"));
    }
}
