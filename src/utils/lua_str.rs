use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if",
    "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

pub fn is_lua_keyword(s: &str) -> bool {
    LUA_KEYWORDS.contains(&s)
}

pub fn is_valid_lua_ident(s: &str) -> bool {
    if s.is_empty() || is_lua_keyword(s) {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first == '_' || first.is_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// 将字符串编码为合法的 Lua 字面量（与原版 `y3.lua.encode(str)` 一致）
pub fn encode_lua_string(value: &str) -> String {
    let nl_num = value.chars().filter(|&c| c == '\n').count();
    let cc_num = value
        .chars()
        .filter(|&c| {
            let code = c as u32;
            (code <= 0x08) || (0x0B..=0x1F).contains(&code) || (0x7F..=0x9F).contains(&code)
        })
        .count();

    if cc_num <= nl_num && nl_num > 0 {
        let mut eq_len = 0;
        loop {
            let end = format!("]{}]", "=".repeat(eq_len));
            if !value.contains(&end) {
                let start = format!("[{}[", "=".repeat(eq_len));
                return format!("{}\n{}{}", start, value, end);
            }
            eq_len += 1;
        }
    } else {
        let single_num = value.chars().filter(|&c| c == '\'').count();
        let double_num = value.chars().filter(|&c| c == '"').count();
        let delim = if double_num > single_num { '\'' } else { '"' };

        let mut out = String::with_capacity(value.len() + 2);
        out.push(delim);
        for c in value.chars() {
            let code = c as u32;
            if c == delim || c == '\\' {
                out.push('\\');
                out.push(c);
            } else if code <= 0x08 || (0x0B..=0x1F).contains(&code) || (0x7F..=0x9F).contains(&code)
            {
                out.push_str(&format!("\\x{:02x}", code));
            } else {
                out.push(c);
            }
        }
        out.push(delim);
        out
    }
}

/// 将任意 JSON Value 编码为 Lua 表达式字符串（与原版 `y3.lua.encode` 一致）
pub fn encode_lua_value(val: &Value) -> String {
    let mut encoder = LuaEncoder::default();
    encoder.encode_value(val);
    encoder.buffer
}

pub struct LuaEncoder {
    pub buffer: String,
    pub newline: String,
    pub indent: String,
    pub depth: usize,
}

impl Default for LuaEncoder {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            newline: "\n".to_string(),
            indent: "    ".to_string(),
            depth: 0,
        }
    }
}

impl LuaEncoder {
    pub fn encode_value(&mut self, val: &Value) {
        match val {
            Value::Null => self.buffer.push_str("nil"),
            Value::Bool(b) => self.buffer.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    self.buffer.push_str(&i.to_string());
                } else if let Some(u) = n.as_u64() {
                    self.buffer.push_str(&u.to_string());
                } else if let Some(f) = n.as_f64() {
                    if f.fract() == 0.0 {
                        self.buffer.push_str(&(f as i64).to_string());
                    } else {
                        let s = f.to_string();
                        if !s.contains('.') && !s.contains('e') {
                            self.buffer.push_str(&format!("{}.0", s));
                        } else {
                            self.buffer.push_str(&s);
                        }
                    }
                }
            }
            Value::String(s) => {
                self.buffer.push_str(&encode_lua_string(s));
            }
            Value::Array(arr) => {
                if arr.is_empty() {
                    self.buffer.push_str("{}");
                } else if arr.len() <= 5 {
                    self.buffer.push_str("{ ");
                    for (i, item) in arr.iter().enumerate() {
                        if i > 0 {
                            self.buffer.push_str(", ");
                        }
                        self.encode_value(item);
                    }
                    self.buffer.push_str(" }");
                } else {
                    self.buffer.push('{');
                    self.buffer.push_str(&self.newline);
                    self.depth += 1;
                    for item in arr {
                        self.buffer.push_str(&self.indent.repeat(self.depth));
                        self.encode_value(item);
                        self.buffer.push(',');
                        self.buffer.push_str(&self.newline);
                    }
                    self.depth -= 1;
                    self.buffer.push_str(&self.indent.repeat(self.depth));
                    self.buffer.push('}');
                }
            }
            Value::Object(map) => {
                self.buffer.push('{');
                self.buffer.push_str(&self.newline);
                self.depth += 1;
                for (k, v) in map {
                    self.buffer.push_str(&self.indent.repeat(self.depth));
                    if is_valid_lua_ident(k) {
                        self.buffer.push_str(k);
                    } else {
                        self.buffer.push('[');
                        self.buffer.push_str(&encode_lua_string(k));
                        self.buffer.push(']');
                    }
                    self.buffer.push_str(" = ");
                    self.encode_value(v);
                    self.buffer.push(',');
                    self.buffer.push_str(&self.newline);
                }
                self.depth -= 1;
                self.buffer.push_str(&self.indent.repeat(self.depth));
                self.buffer.push('}');
            }
        }
    }
}

/// 对应原版 `y3.lua.getValidName` 的作用域命名去重器
#[derive(Default, Clone)]
pub struct ValidNameGenerator {
    forward: HashMap<String, String>,
    backward: HashMap<String, String>,
}

impl ValidNameGenerator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_valid_name(&mut self, name: &str, reserved: Option<&HashSet<String>>) -> String {
        if let Some(existing) = self.forward.get(name) {
            return existing.clone();
        }
        let mut lua_name = String::new();
        if let Some(first) = name.chars().next() {
            if first.is_ascii_digit() {
                lua_name.push('_');
            }
        }
        for c in name.chars() {
            let code = c as u32;
            let is_special = code <= 0x08
                || (0x0B..=0x2F).contains(&code)
                || (0x3A..=0x40).contains(&code)
                || (0x5B..=0x60).contains(&code)
                || (0x7B..=0x9F).contains(&code);
            if is_special {
                lua_name.push('_');
            } else {
                lua_name.push(c);
            }
        }
        let is_reserved = reserved.map(|r| r.contains(&lua_name)).unwrap_or(false);
        if name.is_empty() || is_lua_keyword(&lua_name) || is_reserved {
            lua_name.push('_');
        }
        if self.backward.contains_key(&lua_name) {
            let mut i = 1usize;
            loop {
                let candidate = format!("{}{}", lua_name, i);
                if !self.backward.contains_key(&candidate) {
                    lua_name = candidate;
                    break;
                }
                i += 1;
            }
        }
        self.forward.insert(name.to_string(), lua_name.clone());
        self.backward.insert(lua_name.clone(), name.to_string());
        lua_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_lua_string() {
        assert_eq!(encode_lua_string("hello"), "\"hello\"");
        assert_eq!(encode_lua_string("he\"llo"), "'he\"llo'");
        assert_eq!(encode_lua_string("line1\nline2"), "[[\nline1\nline2]]");
    }

    #[test]
    fn test_valid_name() {
        let mut gen = ValidNameGenerator::new();
        assert_eq!(gen.get_valid_name("123abc", None), "_123abc");
        assert_eq!(gen.get_valid_name("local", None), "local_");
        assert_eq!(gen.get_valid_name("a-b", None), "a_b");
        assert_eq!(gen.get_valid_name("a/b", None), "a_b1");
    }
}
