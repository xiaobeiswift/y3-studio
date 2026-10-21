use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

/// Y3 特有数据节点：精确区分整数、浮点数（即使值为整数如 1.0）、普通数组与 Tuple（`{"__tuple__": true, "items": [...]}`）
#[derive(Debug, Clone, PartialEq)]
pub enum Y3Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Y3Value>),
    Tuple(Vec<Y3Value>),
    Object(BTreeMap<String, Y3Value>),
}

impl Y3Value {
    /// 从标准 `serde_json::Value` 转换，自动识别 `{"__tuple__": true, "items": [...]}`
    pub fn from_json_value(val: &Value) -> Self {
        match val {
            Value::Null => Y3Value::Null,
            Value::Bool(b) => Y3Value::Bool(*b),
            Value::Number(n) => {
                if n.is_f64() && !n.is_i64() && !n.is_u64() {
                    Y3Value::Float(n.as_f64().unwrap_or(0.0))
                } else if let Some(i) = n.as_i64() {
                    Y3Value::Int(i)
                } else if let Some(u) = n.as_u64() {
                    Y3Value::Int(u as i64)
                } else {
                    Y3Value::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            Value::String(s) => Y3Value::String(s.clone()),
            Value::Array(arr) => {
                Y3Value::Array(arr.iter().map(Y3Value::from_json_value).collect())
            }
            Value::Object(map) => {
                if map.get("__tuple__") == Some(&Value::Bool(true)) {
                    if let Some(Value::Array(items)) = map.get("items") {
                        return Y3Value::Tuple(
                            items.iter().map(Y3Value::from_json_value).collect(),
                        );
                    }
                }
                let mut tree = BTreeMap::new();
                for (k, v) in map {
                    tree.insert(k.clone(), Y3Value::from_json_value(v));
                }
                Y3Value::Object(tree)
            }
        }
    }

    /// 转换为展开 Tuple 后的普通 `serde_json::Value`（供业务层与 UI / GMP 读取使用）
    pub fn to_plain_json(&self) -> Value {
        match self {
            Y3Value::Null => Value::Null,
            Y3Value::Bool(b) => Value::Bool(*b),
            Y3Value::Int(i) => Value::Number((*i).into()),
            Y3Value::Float(f) => Number::from_f64(*f)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Y3Value::String(s) => Value::String(s.clone()),
            Y3Value::Array(arr) | Y3Value::Tuple(arr) => {
                Value::Array(arr.iter().map(|v| v.to_plain_json()).collect())
            }
            Y3Value::Object(map) => {
                let mut obj = Map::new();
                for (k, v) in map {
                    obj.insert(k.clone(), v.to_plain_json());
                }
                Value::Object(obj)
            }
        }
    }

    /// 转换为保留 `__tuple__` 结构的 `serde_json::Value`
    pub fn to_raw_json(&self) -> Value {
        match self {
            Y3Value::Null => Value::Null,
            Y3Value::Bool(b) => Value::Bool(*b),
            Y3Value::Int(i) => Value::Number((*i).into()),
            Y3Value::Float(f) => Number::from_f64(*f)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Y3Value::String(s) => Value::String(s.clone()),
            Y3Value::Array(arr) => {
                Value::Array(arr.iter().map(|v| v.to_raw_json()).collect())
            }
            Y3Value::Tuple(items) => {
                let mut obj = Map::new();
                obj.insert("__tuple__".to_string(), Value::Bool(true));
                obj.insert(
                    "items".to_string(),
                    Value::Array(items.iter().map(|v| v.to_raw_json()).collect()),
                );
                Value::Object(obj)
            }
            Y3Value::Object(map) => {
                let mut obj = Map::new();
                for (k, v) in map {
                    obj.insert(k.clone(), v.to_raw_json());
                }
                Value::Object(obj)
            }
        }
    }

    /// 根据新传入的 `serde_json::Value` 更新当前节点，同时保留原有的 Float / Int / Tuple 类型特征（与原版 `Y3Json.updateWith` 一致）
    pub fn update_with(&mut self, new_val: &Value) {
        match (self, new_val) {
            (Y3Value::Float(old_f), Value::Number(n)) => {
                if let Some(f) = n.as_f64() {
                    *old_f = f;
                }
            }
            (Y3Value::Int(old_i), Value::Number(n)) => {
                if let Some(i) = n.as_i64() {
                    *old_i = i;
                } else if let Some(f) = n.as_f64() {
                    *old_i = f as i64;
                }
            }
            (Y3Value::Tuple(old_items), Value::Array(new_arr)) => {
                let mut updated = Vec::with_capacity(new_arr.len());
                for (idx, item) in new_arr.iter().enumerate() {
                    if let Some(mut existing) = old_items.get(idx).cloned() {
                        existing.update_with(item);
                        updated.push(existing);
                    } else {
                        updated.push(Y3Value::from_json_value(item));
                    }
                }
                *old_items = updated;
            }
            (Y3Value::Array(old_items), Value::Array(new_arr)) => {
                let mut updated = Vec::with_capacity(new_arr.len());
                for (idx, item) in new_arr.iter().enumerate() {
                    if let Some(mut existing) = old_items.get(idx).cloned() {
                        existing.update_with(item);
                        updated.push(existing);
                    } else {
                        updated.push(Y3Value::from_json_value(item));
                    }
                }
                *old_items = updated;
            }
            (Y3Value::Object(old_map), Value::Object(new_map)) => {
                if new_map.get("__tuple__") == Some(&Value::Bool(true)) {
                    if let Some(Value::Array(items)) = new_map.get("items") {
                        let tuple_items: Vec<Y3Value> = items.iter().map(Y3Value::from_json_value).collect();
                        *old_map = BTreeMap::new();
                        let _ = tuple_items;
                    }
                }
                // 删除新对象中不存在的键
                let keys_to_remove: Vec<String> = old_map
                    .keys()
                    .filter(|k| !new_map.contains_key(*k))
                    .cloned()
                    .collect();
                for k in keys_to_remove {
                    old_map.remove(&k);
                }
                for (k, v) in new_map {
                    if let Some(existing) = old_map.get_mut(k) {
                        existing.update_with(v);
                    } else {
                        old_map.insert(k.clone(), Y3Value::from_json_value(v));
                    }
                }
            }
            (slot, other) => {
                *slot = Y3Value::from_json_value(other);
            }
        }
    }

    /// 序列化为 Y3 编辑器标准 JSON 文本（4 空格缩进、字典序键、浮点数保留 `.0`、非 ASCII 转 `\uXXXX`）
    pub fn stringify(&self) -> String {
        let mut buf = String::new();
        self.write_formatted(&mut buf, 0);
        buf
    }

    fn write_formatted(&self, buf: &mut String, depth: usize) {
        let indent = "    ".repeat(depth);
        let next_indent = "    ".repeat(depth + 1);
        match self {
            Y3Value::Null => buf.push_str("null"),
            Y3Value::Bool(b) => buf.push_str(if *b { "true" } else { "false" }),
            Y3Value::Int(i) => buf.push_str(&i.to_string()),
            Y3Value::Float(f) => {
                if f.fract() == 0.0 {
                    buf.push_str(&format!("{:.1}", f));
                } else {
                    buf.push_str(&f.to_string());
                }
            }
            Y3Value::String(s) => {
                buf.push_str(&escape_y3_json_string(s));
            }
            Y3Value::Array(arr) => {
                if arr.is_empty() {
                    buf.push_str("[]");
                    return;
                }
                buf.push_str("[\n");
                for (i, item) in arr.iter().enumerate() {
                    buf.push_str(&next_indent);
                    item.write_formatted(buf, depth + 1);
                    if i + 1 < arr.len() {
                        buf.push(',');
                    }
                    buf.push('\n');
                }
                buf.push_str(&indent);
                buf.push(']');
            }
            Y3Value::Tuple(items) => {
                buf.push_str("{\n");
                buf.push_str(&next_indent);
                buf.push_str("\"__tuple__\": true,\n");
                buf.push_str(&next_indent);
                buf.push_str("\"items\": ");
                Y3Value::Array(items.clone()).write_formatted(buf, depth + 1);
                buf.push('\n');
                buf.push_str(&indent);
                buf.push('}');
            }
            Y3Value::Object(map) => {
                if map.is_empty() {
                    buf.push_str("{}");
                    return;
                }
                buf.push_str("{\n");
                let len = map.len();
                for (i, (k, v)) in map.iter().enumerate() {
                    buf.push_str(&next_indent);
                    buf.push_str(&escape_y3_json_string(k));
                    buf.push_str(": ");
                    v.write_formatted(buf, depth + 1);
                    if i + 1 < len {
                        buf.push(',');
                    }
                    buf.push('\n');
                }
                buf.push_str(&indent);
                buf.push('}');
            }
        }
    }
}

/// 解析 Y3 JSON 字符串，精确保留整数与带小数点的浮点数（如 `1.0`）以及 `__tuple__`
pub fn parse_y3_json(input: &str) -> anyhow::Result<Y3Value> {
    let mut parser = Y3JsonParser::new(input);
    parser.parse_root()
}

/// 递归将 `serde_json::Value` 中的 `{"__tuple__": true, "items": [...]}` 转换为普通 `Value::Array`
pub fn convert_tuple_format(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            if map.get("__tuple__") == Some(&Value::Bool(true)) {
                if let Some(Value::Array(items)) = map.get("items") {
                    return Value::Array(items.iter().map(convert_tuple_format).collect());
                }
            }
            let mut out = Map::new();
            for (k, v) in map {
                out.insert(k.clone(), convert_tuple_format(v));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(convert_tuple_format).collect()),
        other => other.clone(),
    }
}

pub fn escape_y3_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for u in s.encode_utf16() {
        match u {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x0C => out.push_str("\\f"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x20..=0x7E => out.push(char::from_u32(u as u32).unwrap()),
            _ => out.push_str(&format!("\\u{:04x}", u)),
        }
    }
    out.push('"');
    out
}

struct Y3JsonParser<'a> {
    chars: &'a [u8],
    pos: usize,
}

impl<'a> Y3JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            chars: input.as_bytes(),
            pos: 0,
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.chars.len() {
            match self.chars[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn parse_root(&mut self) -> anyhow::Result<Y3Value> {
        // 跳过 UTF-8 BOM
        if self.chars.starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.pos = 3;
        }
        self.skip_whitespace();
        let val = self.parse_value()?;
        Ok(val)
    }

    fn parse_value(&mut self) -> anyhow::Result<Y3Value> {
        self.skip_whitespace();
        if self.pos >= self.chars.len() {
            anyhow::bail!("Unexpected EOF in Y3Json");
        }
        match self.chars[self.pos] {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => Ok(Y3Value::String(self.parse_string()?)),
            b't' => {
                if self.chars[self.pos..].starts_with(b"true") {
                    self.pos += 4;
                    Ok(Y3Value::Bool(true))
                } else {
                    anyhow::bail!("Invalid token at {}", self.pos);
                }
            }
            b'f' => {
                if self.chars[self.pos..].starts_with(b"false") {
                    self.pos += 5;
                    Ok(Y3Value::Bool(false))
                } else {
                    anyhow::bail!("Invalid token at {}", self.pos);
                }
            }
            b'n' => {
                if self.chars[self.pos..].starts_with(b"null") {
                    self.pos += 4;
                    Ok(Y3Value::Null)
                } else {
                    anyhow::bail!("Invalid token at {}", self.pos);
                }
            }
            b'-' | b'0'..=b'9' => self.parse_number(),
            c => anyhow::bail!("Unexpected char '{}' at {}", c as char, self.pos),
        }
    }

    fn parse_object(&mut self) -> anyhow::Result<Y3Value> {
        self.pos += 1; // skip '{'
        self.skip_whitespace();
        let mut map = BTreeMap::new();
        if self.pos < self.chars.len() && self.chars[self.pos] == b'}' {
            self.pos += 1;
            return Ok(Y3Value::Object(map));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.pos >= self.chars.len() || self.chars[self.pos] != b':' {
                anyhow::bail!("Expected ':' at {}", self.pos);
            }
            self.pos += 1;
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_whitespace();
            if self.pos >= self.chars.len() {
                anyhow::bail!("Unexpected EOF in object");
            }
            match self.chars[self.pos] {
                b',' => {
                    self.pos += 1;
                    self.skip_whitespace();
                    if self.pos < self.chars.len() && self.chars[self.pos] == b'}' {
                        self.pos += 1;
                        break;
                    }
                }
                b'}' => {
                    self.pos += 1;
                    break;
                }
                c => anyhow::bail!("Unexpected char '{}' in object at {}", c as char, self.pos),
            }
        }
        if map.get("__tuple__") == Some(&Y3Value::Bool(true)) {
            if let Some(Y3Value::Array(items)) = map.remove("items") {
                return Ok(Y3Value::Tuple(items));
            }
        }
        Ok(Y3Value::Object(map))
    }

    fn parse_array(&mut self) -> anyhow::Result<Y3Value> {
        self.pos += 1; // skip '['
        self.skip_whitespace();
        let mut arr = Vec::new();
        if self.pos < self.chars.len() && self.chars[self.pos] == b']' {
            self.pos += 1;
            return Ok(Y3Value::Array(arr));
        }
        loop {
            let val = self.parse_value()?;
            arr.push(val);
            self.skip_whitespace();
            if self.pos >= self.chars.len() {
                anyhow::bail!("Unexpected EOF in array");
            }
            match self.chars[self.pos] {
                b',' => {
                    self.pos += 1;
                    self.skip_whitespace();
                    if self.pos < self.chars.len() && self.chars[self.pos] == b']' {
                        self.pos += 1;
                        break;
                    }
                }
                b']' => {
                    self.pos += 1;
                    break;
                }
                c => anyhow::bail!("Unexpected char '{}' in array at {}", c as char, self.pos),
            }
        }
        Ok(Y3Value::Array(arr))
    }

    fn parse_string(&mut self) -> anyhow::Result<String> {
        if self.pos >= self.chars.len() || self.chars[self.pos] != b'"' {
            anyhow::bail!("Expected '\"' at {}", self.pos);
        }
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.chars.len() {
            match self.chars[self.pos] {
                b'\\' => self.pos += 2,
                b'"' => {
                    self.pos += 1;
                    let raw = std::str::from_utf8(&self.chars[start..self.pos])?;
                    let parsed: String = serde_json::from_str(raw)?;
                    return Ok(parsed);
                }
                _ => self.pos += 1,
            }
        }
        anyhow::bail!("Unterminated string starting at {}", start)
    }

    fn parse_number(&mut self) -> anyhow::Result<Y3Value> {
        let start = self.pos;
        let mut is_float = false;
        if self.chars[self.pos] == b'-' {
            self.pos += 1;
        }
        while self.pos < self.chars.len() {
            match self.chars[self.pos] {
                b'0'..=b'9' => self.pos += 1,
                b'.' | b'e' | b'E' | b'+' | b'-' => {
                    is_float = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        let raw = std::str::from_utf8(&self.chars[start..self.pos])?;
        if is_float {
            let f: f64 = raw.parse()?;
            Ok(Y3Value::Float(f))
        } else if let Ok(i) = raw.parse::<i64>() {
            Ok(Y3Value::Int(i))
        } else {
            let f: f64 = raw.parse()?;
            Ok(Y3Value::Float(f))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_y3_json_roundtrip() {
        let raw = r#"{
    "a_float": 1.0,
    "b_int": 42,
    "c_tuple": {
        "__tuple__": true,
        "items": [
            10,
            "中文"
        ]
    }
}"#;
        let parsed = parse_y3_json(raw).unwrap();
        let plain = parsed.to_plain_json();
        assert_eq!(plain["c_tuple"], serde_json::json!([10, "中文"]));

        let serialized = parsed.stringify();
        assert!(serialized.contains("\"a_float\": 1.0"));
        assert!(serialized.contains("\"b_int\": 42"));
        assert!(serialized.contains("\"__tuple__\": true"));
        assert!(serialized.contains("\\u4e2d\\u6587"));
    }
}
