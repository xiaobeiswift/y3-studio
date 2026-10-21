use crate::utils::murmur::murmur_hash3_ts_compat;
use crate::utils::y3json::{parse_y3_json, Y3Value};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 管理地图或项目级的 `zhlanguage.json` 多语言词条表
pub struct LanguageManager {
    pub file_path: PathBuf,
    pub entries: BTreeMap<String, String>,
}

impl LanguageManager {
    /// 从语言目录（`{project}/language` 或 `{trigger_map}`）加载 `zhlanguage.json`
    pub fn load(lang_dir: &Path) -> Self {
        let file_path = lang_dir.join("zhlanguage.json");
        let mut entries = BTreeMap::new();
        if let Ok(content) = fs::read_to_string(&file_path) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                if let Some(obj) = val.as_object() {
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            entries.insert(k.clone(), s.to_string());
                        }
                    }
                }
            }
        }
        Self { file_path, entries }
    }

    /// 根据多语言 key（如 `-123456` 或 `"123456"`）查询中文文本
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|s| s.as_str())
    }

    /// 设置物编指定对象、指定字段的本地化文本，返回写入物编 JSON 的多语言 key（i32 或 u32 形式）
    pub fn set_object_field_text(&mut self, obj_key: u64, field_name: &str, text: &str) -> i64 {
        let hash_input = format!("{}_{}", obj_key, field_name);
        let h_u32 = murmur_hash3_ts_compat(&hash_input, 0);
        // 原版 TS 中 `hash(key)` 返回 `h1 >>> 0`，但也可能为有符号整数字符串，统一使用 `h_u32 as i32` 或 `h_u32`
        let key_str = h_u32.to_string();
        self.entries.insert(key_str, text.to_string());
        h_u32 as i64
    }

    /// 解析物编字段中的名称或描述（支持整数哈希键、字符串哈希键或直接内联字符串）
    pub fn resolve_field_value(&self, raw_val: &Value) -> Option<String> {
        match raw_val {
            Value::Number(n) => {
                let s = n.to_string();
                self.get(&s).map(|v| v.to_string()).or(Some(s))
            }
            Value::String(s) => self
                .get(s)
                .map(|v| v.to_string())
                .or_else(|| Some(s.clone())),
            _ => None,
        }
    }

    /// 将修改后的 `zhlanguage.json` 保存到磁盘（使用 Y3Json 格式）
    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut map = BTreeMap::new();
        for (k, v) in &self.entries {
            map.insert(k.clone(), Y3Value::String(v.clone()));
        }
        let y3_val = Y3Value::Object(map);
        fs::write(&self.file_path, y3_val.stringify())?;
        Ok(())
    }

    /// 解析任意 JSON 文件为 Y3Value
    pub fn load_y3_file(path: &Path) -> anyhow::Result<Y3Value> {
        let content = fs::read_to_string(path)?;
        parse_y3_json(&content)
    }
}
