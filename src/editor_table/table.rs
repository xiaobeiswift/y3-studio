use crate::editor_table::language::LanguageManager;
use crate::env::ProjectContext;
use crate::utils::y3json::{parse_y3_json, Y3Value};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

// 内嵌原版插件的物编默认 JSON 模板
const TPL_UNIT: &str = include_str!("../../y3-helper-vscode/template/json_template/unit.json");
const TPL_ABILITY: &str =
    include_str!("../../y3-helper-vscode/template/json_template/ability.json");
const TPL_MODIFIER: &str =
    include_str!("../../y3-helper-vscode/template/json_template/modifier.json");
const TPL_ITEM: &str = include_str!("../../y3-helper-vscode/template/json_template/item.json");
const TPL_PROJECTILE: &str =
    include_str!("../../y3-helper-vscode/template/json_template/projectile.json");
const TPL_DESTRUCTIBLE: &str =
    include_str!("../../y3-helper-vscode/template/json_template/destructible.json");
const TPL_DECORATION: &str =
    include_str!("../../y3-helper-vscode/template/json_template/decoration.json");

// 内嵌原版插件的物编字段元数据
const META_UNIT: &str = include_str!("../../y3-helper-vscode/src/helper_meta/editor/unit.json");
const META_ABILITY: &str =
    include_str!("../../y3-helper-vscode/src/helper_meta/editor/ability.json");
const META_MODIFIER: &str =
    include_str!("../../y3-helper-vscode/src/helper_meta/editor/modifier.json");
const META_ITEM: &str = include_str!("../../y3-helper-vscode/src/helper_meta/editor/item.json");
const META_PROJECTILE: &str =
    include_str!("../../y3-helper-vscode/src/helper_meta/editor/projectile.json");
const META_DESTRUCTIBLE: &str =
    include_str!("../../y3-helper-vscode/src/helper_meta/editor/destructible.json");
const META_DECORATION: &str =
    include_str!("../../y3-helper-vscode/src/helper_meta/editor/decoration.json");
const META_SOUND: &str = include_str!("../../y3-helper-vscode/src/helper_meta/editor/sound.json");
const META_TECH: &str = include_str!("../../y3-helper-vscode/src/helper_meta/editor/tech.json");

/// Y3 9 大物编表类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TableType {
    Unit,
    Sound,
    Ability,
    Decoration,
    Destructible,
    Item,
    Modifier,
    Projectile,
    Technology,
}

impl TableType {
    pub const ALL: &'static [TableType] = &[
        TableType::Unit,
        TableType::Sound,
        TableType::Ability,
        TableType::Decoration,
        TableType::Destructible,
        TableType::Item,
        TableType::Modifier,
        TableType::Projectile,
        TableType::Technology,
    ];

    /// 支持中文名称、目录名称或英文短名称解析
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_lowercase().as_str() {
            "单位" | "editorunit" | "editor_unit" | "unit" => Some(Self::Unit),
            "声音" | "soundall" | "sound_all" | "sound" => Some(Self::Sound),
            "技能" | "abilityall" | "ability_all" | "ability" => Some(Self::Ability),
            "装饰物" | "editordecoration" | "editor_decoration" | "decoration" => {
                Some(Self::Decoration)
            }
            "可破坏物" | "editordestructible" | "editor_destructible" | "destructible" => {
                Some(Self::Destructible)
            }
            "物品" | "editoritem" | "editor_item" | "item" => Some(Self::Item),
            "魔法效果" | "modifierall" | "modifier_all" | "modifier" | "buff" => {
                Some(Self::Modifier)
            }
            "投射物" | "projectileall" | "projectile_all" | "projectile" => {
                Some(Self::Projectile)
            }
            "科技" | "technologyall" | "technology_all" | "technology" | "tech" => {
                Some(Self::Technology)
            }
            _ => None,
        }
    }

    pub fn name_cn(&self) -> &'static str {
        match self {
            Self::Unit => "单位",
            Self::Sound => "声音",
            Self::Ability => "技能",
            Self::Decoration => "装饰物",
            Self::Destructible => "可破坏物",
            Self::Item => "物品",
            Self::Modifier => "魔法效果",
            Self::Projectile => "投射物",
            Self::Technology => "科技",
        }
    }

    pub fn folder_name(&self) -> &'static str {
        match self {
            Self::Unit => "editorunit",
            Self::Sound => "soundall",
            Self::Ability => "abilityall",
            Self::Decoration => "editordecoration",
            Self::Destructible => "editordestructible",
            Self::Item => "editoritem",
            Self::Modifier => "modifierall",
            Self::Projectile => "projectileall",
            Self::Technology => "technologyall",
        }
    }

    pub fn short_en(&self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Sound => "sound",
            Self::Ability => "ability",
            Self::Decoration => "decoration",
            Self::Destructible => "destructible",
            Self::Item => "item",
            Self::Modifier => "modifier",
            Self::Projectile => "projectile",
            Self::Technology => "technology",
        }
    }

    pub fn default_template_str(&self) -> &'static str {
        match self {
            Self::Unit => TPL_UNIT,
            Self::Ability => TPL_ABILITY,
            Self::Modifier => TPL_MODIFIER,
            Self::Item => TPL_ITEM,
            Self::Projectile => TPL_PROJECTILE,
            Self::Destructible => TPL_DESTRUCTIBLE,
            Self::Decoration => TPL_DECORATION,
            Self::Sound => r#"{"_custom_": true, "key": 134217728, "uid": "134217728", "name": 0, "kv": {}}"#,
            Self::Technology => r#"{"_custom_": true, "key": 134217728, "uid": "134217728", "name": 0, "kv": {}}"#,
        }
    }

    pub fn field_meta_json(&self) -> Value {
        let raw = match self {
            Self::Unit => META_UNIT,
            Self::Ability => META_ABILITY,
            Self::Modifier => META_MODIFIER,
            Self::Item => META_ITEM,
            Self::Projectile => META_PROJECTILE,
            Self::Destructible => META_DESTRUCTIBLE,
            Self::Decoration => META_DECORATION,
            Self::Sound => META_SOUND,
            Self::Technology => META_TECH,
        };
        serde_json::from_str(raw).unwrap_or_else(|_| json!({}))
    }
}

/// 单个物编对象摘要信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSummary {
    pub key: u64,
    pub name: String,
    pub description: Option<String>,
    pub file_path: PathBuf,
}

/// 单个物编对象详情（包含已解析本地化字段及原始 Y3Value）
pub struct EditorObject {
    pub table_type: TableType,
    pub key: u64,
    pub name: String,
    pub file_path: PathBuf,
    pub y3_value: Y3Value,
    pub json_data: Value,
}

/// 物编表管理器（针对指定项目与指定子地图）
pub struct EditorTable {
    pub table_type: TableType,
    pub table_dir: PathBuf,
    pub lang_dir: PathBuf,
    pub field_meta: Value,
}

impl EditorTable {
    /// 打开指定项目、指定子地图下的物编表（自动处理 `use_main_level_trigger_and_object`）
    pub fn open(
        project: &ProjectContext,
        map_name: Option<&str>,
        table_type: TableType,
    ) -> anyhow::Result<Self> {
        let trigger_map = project.trigger_map_for(map_name)?;
        let table_dir = trigger_map
            .map_path
            .join("editor_table")
            .join(table_type.folder_name());
        let lang_dir = project.language_dir_for(map_name)?;
        let field_meta = table_type.field_meta_json();

        Ok(Self {
            table_type,
            table_dir,
            lang_dir,
            field_meta,
        })
    }

    /// 获取当前物编表下所有对象的 ID 列表（升序排列）
    pub fn list_keys(&self) -> anyhow::Result<Vec<u64>> {
        let mut keys = Vec::new();
        if !self.table_dir.is_dir() {
            return Ok(keys);
        }
        for entry in fs::read_dir(&self.table_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(id) = stem.parse::<u64>() {
                    keys.push(id);
                }
            }
        }
        keys.sort_unstable();
        Ok(keys)
    }

    /// 获取当前物编表下所有对象的摘要列表（含中文名称与描述）
    pub fn list_summaries(&self) -> anyhow::Result<Vec<ObjectSummary>> {
        let lang = LanguageManager::load(&self.lang_dir);
        let keys = self.list_keys()?;
        let mut list = Vec::with_capacity(keys.len());

        for key in keys {
            let file_path = self.table_dir.join(format!("{}.json", key));
            let content = match fs::read_to_string(&file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let val: Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let name = val
                .get("name")
                .and_then(|v| lang.resolve_field_value(v))
                .unwrap_or_else(|| key.to_string());
            let description = val
                .get("description")
                .or_else(|| val.get("desc"))
                .and_then(|v| lang.resolve_field_value(v));
            list.push(ObjectSummary {
                key,
                name,
                description,
                file_path,
            });
        }
        Ok(list)
    }

    /// 读取单个物编对象（同时将本地化哈希键翻译为可读中文，并展开 `__tuple__` 方便业务读取）
    pub fn get(&self, key: u64) -> anyhow::Result<EditorObject> {
        let file_path = self.table_dir.join(format!("{}.json", key));
        if !file_path.is_file() {
            anyhow::bail!(
                "物编表 [{}] 中不存在 key={} ({})",
                self.table_type.name_cn(),
                key,
                file_path.display()
            );
        }
        let content = fs::read_to_string(&file_path)?;
        let y3_value = parse_y3_json(&content)?;
        let mut json_data = y3_value.to_plain_json();
        let lang = LanguageManager::load(&self.lang_dir);

        // 根据字段元数据将 Language 类型字段解析为中文字符串
        if let (Some(obj), Some(meta_obj)) = (json_data.as_object_mut(), self.field_meta.as_object())
        {
            for (field, field_info) in meta_obj {
                let is_lang = field_info
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| t == "Language")
                    .unwrap_or(false)
                    || field == "name"
                    || field == "description";
                if is_lang {
                    if let Some(raw_val) = obj.get(field) {
                        if let Some(resolved) = lang.resolve_field_value(raw_val) {
                            obj.insert(field.clone(), Value::String(resolved));
                        }
                    }
                }
            }
        }

        let name = json_data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(EditorObject {
            table_type: self.table_type,
            key,
            name,
            file_path,
            y3_value,
            json_data,
        })
    }

    /// 生成一个新的空闲物编 key（默认从 134217728 起递增，确保不与已有 key 冲突）
    pub fn next_available_key(&self) -> anyhow::Result<u64> {
        let existing = self.list_keys()?;
        let mut candidate: u64 = 134217728;
        for k in existing {
            if k >= candidate {
                candidate = k + 1;
            }
        }
        Ok(candidate)
    }

    /// 创建或覆盖一个物编对象：
    /// - 若指定 `copy_from_key` 且存在，则以该物编为基底克隆；否则使用内置官方模板
    /// - 自动更新 `key`、`uid`、本地化 `name` 及传入的 `fields` 和 `kv`
    pub fn create(
        &self,
        key: Option<u64>,
        name: &str,
        copy_from_key: Option<u64>,
        fields: Option<&Map<String, Value>>,
        kv: Option<&Map<String, Value>>,
    ) -> anyhow::Result<EditorObject> {
        fs::create_dir_all(&self.table_dir)?;
        let target_key = match key {
            Some(k) => k,
            None => self.next_available_key()?,
        };

        let mut y3_val = if let Some(src_key) = copy_from_key {
            let src_path = self.table_dir.join(format!("{}.json", src_key));
            if src_path.is_file() {
                parse_y3_json(&fs::read_to_string(&src_path)?)?
            } else {
                parse_y3_json(self.table_type.default_template_str())?
            }
        } else {
            parse_y3_json(self.table_type.default_template_str())?
        };

        let mut lang = LanguageManager::load(&self.lang_dir);

        if let Y3Value::Object(ref mut map) = y3_val {
            map.insert("_custom_".to_string(), Y3Value::Bool(true));
            map.insert("key".to_string(), Y3Value::Int(target_key as i64));
            map.insert("uid".to_string(), Y3Value::String(target_key.to_string()));

            let name_hash = lang.set_object_field_text(target_key, "name", name);
            map.insert("name".to_string(), Y3Value::Int(name_hash));
        }

        self.apply_updates(&mut y3_val, &mut lang, target_key, fields, kv)?;
        lang.save()?;

        let file_path = self.table_dir.join(format!("{}.json", target_key));
        fs::write(&file_path, y3_val.stringify())?;

        self.get(target_key)
    }

    /// 修改已有物编对象的字段与 `kv` 自定义属性
    pub fn update(
        &self,
        key: u64,
        fields: Option<&Map<String, Value>>,
        kv: Option<&Map<String, Value>>,
    ) -> anyhow::Result<EditorObject> {
        let file_path = self.table_dir.join(format!("{}.json", key));
        if !file_path.is_file() {
            anyhow::bail!("物编对象不存在: {} (key={})", self.table_type.name_cn(), key);
        }
        let mut y3_val = parse_y3_json(&fs::read_to_string(&file_path)?)?;
        let mut lang = LanguageManager::load(&self.lang_dir);

        self.apply_updates(&mut y3_val, &mut lang, key, fields, kv)?;
        lang.save()?;
        fs::write(&file_path, y3_val.stringify())?;

        self.get(key)
    }

    /// 删除指定物编对象
    pub fn delete(&self, key: u64) -> anyhow::Result<bool> {
        let file_path = self.table_dir.join(format!("{}.json", key));
        if file_path.is_file() {
            fs::remove_file(&file_path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn apply_updates(
        &self,
        y3_val: &mut Y3Value,
        lang: &mut LanguageManager,
        key: u64,
        fields: Option<&Map<String, Value>>,
        kv: Option<&Map<String, Value>>,
    ) -> anyhow::Result<()> {
        let meta_obj = self.field_meta.as_object();

        // 1. 构建中文字段描述 -> 英文字段名的映射（方便 Excel 导表或用户直接用中文属性名修改）
        let mut desc_to_key = BTreeMap::new();
        if let Some(m) = meta_obj {
            for (k, info) in m {
                if let Some(desc) = info.get("desc").and_then(|v| v.as_str()) {
                    desc_to_key.insert(desc.to_string(), k.clone());
                }
            }
        }

        if let Y3Value::Object(ref mut obj_map) = y3_val {
            if let Some(f_map) = fields {
                for (raw_field, new_val) in f_map {
                    let field_key = desc_to_key
                        .get(raw_field)
                        .cloned()
                        .unwrap_or_else(|| raw_field.clone());

                    let is_lang = meta_obj
                        .and_then(|m| m.get(&field_key))
                        .and_then(|info| info.get("type"))
                        .and_then(|v| v.as_str())
                        .map(|t| t == "Language")
                        .unwrap_or(false)
                        || field_key == "name"
                        || field_key == "description";

                    if is_lang {
                        if let Some(text) = new_val.as_str() {
                            let h = lang.set_object_field_text(key, &field_key, text);
                            obj_map.insert(field_key, Y3Value::Int(h));
                            continue;
                        }
                    }

                    if let Some(existing) = obj_map.get_mut(&field_key) {
                        existing.update_with(new_val);
                    } else {
                        obj_map.insert(field_key, Y3Value::from_json_value(new_val));
                    }
                }
            }

            // 2. 更新自定义属性 `kv`（与原版 `editorData.ts` 的 `kv` 包装结构一致）
            if let Some(kv_updates) = kv {
                let kv_entry = obj_map
                    .entry("kv".to_string())
                    .or_insert_with(|| Y3Value::Object(BTreeMap::new()));
                if let Y3Value::Object(ref mut kv_map) = kv_entry {
                    for (k, v) in kv_updates {
                        if v.is_null() {
                            kv_map.remove(k);
                        } else {
                            let wrapped = Self::wrap_kv_entry(k, v);
                            kv_map.insert(k.clone(), wrapped);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn wrap_kv_entry(key: &str, val: &Value) -> Y3Value {
        let mut entry = BTreeMap::new();
        entry.insert("annotation".to_string(), Y3Value::String(String::new()));
        entry.insert("desc".to_string(), Y3Value::String(String::new()));
        entry.insert("etype".to_string(), Y3Value::Int(0));
        entry.insert("key".to_string(), Y3Value::String(key.to_string()));
        entry.insert("prop_cls".to_string(), Y3Value::String("PInt".to_string()));
        entry.insert("remark".to_string(), Y3Value::String(String::new()));
        entry.insert("show_in_attr".to_string(), Y3Value::Bool(true));
        entry.insert("sort".to_string(), Y3Value::Int(0));

        match val {
            Value::Bool(b) => {
                entry.insert("etype".to_string(), Y3Value::Int(2));
                entry.insert("prop_cls".to_string(), Y3Value::String("PBool".to_string()));
                entry.insert("value".to_string(), Y3Value::Bool(*b));
            }
            Value::Number(n) => {
                if n.is_f64() && !n.is_i64() && !n.is_u64() {
                    entry.insert("etype".to_string(), Y3Value::Int(1));
                    entry.insert(
                        "prop_cls".to_string(),
                        Y3Value::String("PFloat".to_string()),
                    );
                    entry.insert("value".to_string(), Y3Value::Float(n.as_f64().unwrap_or(0.0)));
                } else {
                    entry.insert("etype".to_string(), Y3Value::Int(0));
                    entry.insert("prop_cls".to_string(), Y3Value::String("PInt".to_string()));
                    entry.insert("value".to_string(), Y3Value::Int(n.as_i64().unwrap_or(0)));
                }
            }
            Value::String(s) => {
                entry.insert("etype".to_string(), Y3Value::Int(23));
                entry.insert("prop_cls".to_string(), Y3Value::String("PText".to_string()));
                entry.insert("value".to_string(), Y3Value::String(s.clone()));
            }
            other => {
                entry.insert("value".to_string(), Y3Value::from_json_value(other));
            }
        }
        Y3Value::Object(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editor_table_crud() {
        let tmp = tempfile::tempdir().unwrap();
        let proj_dir = tmp.path().join("Proj");
        fs::create_dir_all(proj_dir.join("maps/EntryMap/script")).unwrap();
        fs::write(
            proj_dir.join("header.project"),
            r#"{"entry_map": {"id": 1}}"#,
        )
        .unwrap();
        fs::write(proj_dir.join("maps/EntryMap/header.map"), r#"{"id": 1}"#).unwrap();

        let ctx = ProjectContext::load_from_path(&proj_dir).unwrap();
        let table = EditorTable::open(&ctx, None, TableType::Unit).unwrap();

        let mut fields = Map::new();
        fields.insert("ori_bits".to_string(), json!(10));
        let mut kv = Map::new();
        kv.insert("atk_speed_bonus".to_string(), json!(1.5));

        let created = table
            .create(Some(134299999), "火焰巨龙", None, Some(&fields), Some(&kv))
            .unwrap();
        assert_eq!(created.key, 134299999);
        assert_eq!(created.name, "火焰巨龙");

        let summaries = table.list_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "火焰巨龙");

        // 更新名称与属性
        let mut update_fields = Map::new();
        update_fields.insert("name".to_string(), json!("冰霜巨龙"));
        let updated = table
            .update(134299999, Some(&update_fields), None)
            .unwrap();
        assert_eq!(updated.name, "冰霜巨龙");

        assert!(table.delete(134299999).unwrap());
        assert!(table.list_keys().unwrap().is_empty());
    }
}
