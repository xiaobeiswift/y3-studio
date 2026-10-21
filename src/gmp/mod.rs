use crate::utils::y3json::convert_tuple_format;
use crate::utils::zstd_frame::{compress_zstd, decompress_zstd};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const PREFAB_SECTIONS: &[(&str, &str)] = &[
    ("editor_unit", "editorunit"),
    ("editor_decoration", "editordecoration"),
    ("ability_all", "abilityall"),
    ("modifier_all", "modifierall"),
    ("projectile_all", "projectileall"),
    ("technology_all", "technologyall"),
    ("store_all", "storeall"),
    ("editor_item", "editoritem"),
    ("editor_destructible", "editordestructible"),
    ("sound_all", "soundall"),
    ("state_all", "stateall"),
];

pub const ALL_SECTIONS_ORDER: &[&str] = &[
    "desc",
    "tech_data",
    "user_data",
    "camp_info",
    "tags_data",
    "editor_mode_info",
    "object_editor_template",
    "modifier_all",
    "sound_all",
    "technology_all",
    "ability_all",
    "projectile_all",
    "store_all",
    "state_all",
    "editor_unit",
    "editor_decoration",
    "editor_item",
    "editor_destructible",
    "physic_object",
    "physics_object_logic",
    "editor_language_us",
    "editor_language_zh",
    "editor_language_tw",
    "editor_language_fr",
    "editor_language_es",
    "editor_language_kr",
    "editor_language_jp",
];

pub const BIT_CONTAINS_BINARY: u32 = 0x8000_0000;
pub const OLD_SECTION_ID_MAX: u16 = 200;
pub const UI_SECTION_ID: u16 = 10;
pub const US_LANGUAGE_SECTION_ID: u16 = 49;
pub const ZH_LANGUAGE_SECTION_ID: u16 = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmpSaveOptions {
    pub update_prefabs: bool,
    pub update_ui: bool,
    pub update_us_language: bool,
    pub update_zh_language: bool,
    pub output_path: Option<PathBuf>,
}

impl Default for GmpSaveOptions {
    fn default() -> Self {
        Self {
            update_prefabs: true,
            update_ui: true,
            update_us_language: true,
            update_zh_language: true,
            output_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmpSaveResult {
    pub success: bool,
    pub message: String,
    pub sections_replaced: Vec<String>,
    pub ui_section_size: Option<usize>,
    pub output_size: usize,
}

#[derive(Debug, Clone)]
pub struct GmpHeader {
    pub uuid: [u8; 16],
    pub version: u16,
    pub pack_size: u32,
    pub create_stamp: String,
    pub editor_pack_stamp: String,
}

#[derive(Debug, Clone)]
pub struct SectionData {
    pub index: u16,
    pub body: Vec<u8>,
    pub is_old_section: bool,
    pub bin_table: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct GmpParser {
    pub header: GmpHeader,
    pub sections: BTreeMap<u16, SectionData>,
}

impl GmpParser {
    pub fn parse_file(path: &Path) -> anyhow::Result<Self> {
        let bytes = fs::read(path)?;
        Self::parse_bytes(&bytes)
    }

    pub fn parse_bytes(buf: &[u8]) -> anyhow::Result<Self> {
        if buf.len() < 22 {
            anyhow::bail!("GMP 数据过短 (< 22 bytes)");
        }
        let mut offset = 0usize;
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&buf[offset..offset + 16]);
        offset += 16;

        let version = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
        offset += 2;
        let pack_size = u32::from_be_bytes([
            buf[offset],
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
        ]);
        offset += 4;

        let mut create_stamp = String::new();
        let mut editor_pack_stamp = String::new();
        if version >= 5 {
            if buf.len() < offset + 24 {
                anyhow::bail!("GMP v5+ Header 缺少时间戳字段");
            }
            create_stamp = String::from_utf8_lossy(&buf[offset..offset + 12])
                .trim_matches('\0')
                .to_string();
            offset += 12;
            editor_pack_stamp = String::from_utf8_lossy(&buf[offset..offset + 12])
                .trim_matches('\0')
                .to_string();
            offset += 12;
        }

        if buf.len() < offset + 2 {
            anyhow::bail!("GMP 缺少 Section 数量字段");
        }
        let sec_cnt = u16::from_be_bytes([buf[offset], buf[offset + 1]]) as usize;
        offset += 2;

        let mut headers = Vec::with_capacity(sec_cnt);
        for _ in 0..sec_cnt {
            if buf.len() < offset + 6 {
                anyhow::bail!("GMP Section Header 截断");
            }
            let sec_idx = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
            let raw_pack_size = u32::from_be_bytes([
                buf[offset + 2],
                buf[offset + 3],
                buf[offset + 4],
                buf[offset + 5],
            ]);
            offset += 6;

            let is_old = sec_idx < OLD_SECTION_ID_MAX;
            let (body_size, has_bin) = if is_old {
                (raw_pack_size as usize, false)
            } else {
                let has_bin = (raw_pack_size & BIT_CONTAINS_BINARY) != 0;
                let sz = (raw_pack_size & !BIT_CONTAINS_BINARY) as usize;
                (sz, has_bin)
            };
            headers.push((sec_idx, body_size, is_old, has_bin));
        }

        let mut sections = BTreeMap::new();
        for (sec_idx, body_size, is_old, has_bin) in headers {
            if buf.len() < offset + body_size {
                anyhow::bail!("GMP Section {} Body 截断", sec_idx);
            }
            let body = buf[offset..offset + body_size].to_vec();
            offset += body_size;

            let bin_table = if has_bin {
                if buf.len() < offset + 4 {
                    anyhow::bail!("GMP Section {} BinTable 长度截断", sec_idx);
                }
                let bin_sz = u32::from_be_bytes([
                    buf[offset],
                    buf[offset + 1],
                    buf[offset + 2],
                    buf[offset + 3],
                ]) as usize;
                offset += 4;
                if buf.len() < offset + bin_sz {
                    anyhow::bail!("GMP Section {} BinTable 内容截断", sec_idx);
                }
                let data = buf[offset..offset + bin_sz].to_vec();
                offset += bin_sz;
                Some(data)
            } else {
                None
            };

            sections.insert(
                sec_idx,
                SectionData {
                    index: sec_idx,
                    body,
                    is_old_section: is_old,
                    bin_table,
                },
            );
        }

        Ok(Self {
            header: GmpHeader {
                uuid,
                version,
                pack_size,
                create_stamp,
                editor_pack_stamp,
            },
            sections,
        })
    }

    pub fn build_bytes(&self) -> Vec<u8> {
        let mut total_pack_size: u32 = 2; // sec_cnt
        for sec in self.sections.values() {
            let mut body_sz = sec.body.len() as u32;
            if !sec.is_old_section {
                if let Some(ref bin) = sec.bin_table {
                    body_sz += 4 + bin.len() as u32;
                }
            }
            total_pack_size += 6 + body_sz;
        }

        let mut out = Vec::with_capacity(total_pack_size as usize + 64);
        out.extend_from_slice(&self.header.uuid);
        out.extend_from_slice(&self.header.version.to_be_bytes());
        out.extend_from_slice(&total_pack_size.to_be_bytes());

        if self.header.version >= 5 {
            let mut c_buf = [0u8; 12];
            let c_bytes = self.header.create_stamp.as_bytes();
            let c_len = c_bytes.len().min(12);
            c_buf[..c_len].copy_from_slice(&c_bytes[..c_len]);
            out.extend_from_slice(&c_buf);

            let mut e_buf = [0u8; 12];
            let e_bytes = self.header.editor_pack_stamp.as_bytes();
            let e_len = e_bytes.len().min(12);
            e_buf[..e_len].copy_from_slice(&e_bytes[..e_len]);
            out.extend_from_slice(&e_buf);
        }

        let sec_cnt = self.sections.len() as u16;
        out.extend_from_slice(&sec_cnt.to_be_bytes());

        for (&sec_idx, sec) in &self.sections {
            out.extend_from_slice(&sec_idx.to_be_bytes());
            let mut raw_len = sec.body.len() as u32;
            if !sec.is_old_section && sec.bin_table.is_some() {
                raw_len |= BIT_CONTAINS_BINARY;
            }
            out.extend_from_slice(&raw_len.to_be_bytes());
        }

        for sec in self.sections.values() {
            out.extend_from_slice(&sec.body);
            if !sec.is_old_section {
                if let Some(ref bin) = sec.bin_table {
                    out.extend_from_slice(&(bin.len() as u32).to_be_bytes());
                    out.extend_from_slice(bin);
                }
            }
        }

        out
    }
}

/// 生成 Section 名称到 16 位索引的映射（与原版 `genSectionIdx` 100% 一致）
pub fn get_prefab_section_index_map() -> BTreeMap<String, u16> {
    let mut all_indexes = BTreeMap::new();
    let mut used = HashSet::new();

    for &sec_name in ALL_SECTIONS_ORDER {
        let mut hasher = Md5::new();
        hasher.update(sec_name.as_bytes());
        let hash = hasher.finalize(); // 16 bytes

        let mut big_part = (hash[12] ^ hash[14]) as u16;
        if big_part == 0 {
            big_part = hash[12] as u16;
        }
        if big_part == 0 {
            big_part = hash[14] as u16;
        }
        if big_part == 0 {
            big_part = 1;
        }

        let small_part = (hash[13] ^ hash[15]) as u16;
        let mut idx = (big_part << 8) | small_part;
        while used.contains(&idx) {
            idx = idx.wrapping_add(1);
        }
        used.insert(idx);
        all_indexes.insert(sec_name.to_string(), idx);
    }

    let mut prefab_map = BTreeMap::new();
    for &(sec_name, _) in PREFAB_SECTIONS {
        if let Some(&idx) = all_indexes.get(sec_name) {
            prefab_map.insert(sec_name.to_string(), idx);
        }
    }
    prefab_map
}

/// 新版 Section 打包：`dict -> JSON string -> msgpack -> zstd`
pub fn pack_section_new_style(data: &Value) -> anyhow::Result<Vec<u8>> {
    let json_str = serde_json::to_string(data)?;
    let msgpacked = rmp_serde::to_vec(&json_str)?;
    Ok(compress_zstd(&msgpacked))
}

/// 新版 Section 解包：`zstd -> msgpack -> JSON string -> Value`
pub fn unpack_section_new_style(compressed: &[u8]) -> anyhow::Result<Value> {
    let msgpacked = decompress_zstd(compressed)?;
    let json_str: String = rmp_serde::from_slice(&msgpacked)?;
    let val: Value = serde_json::from_str(&json_str)?;
    Ok(val)
}

/// 旧版 Section 打包：`dict -> msgpack -> zstd`
pub fn pack_section_old_style(data: &Value) -> anyhow::Result<Vec<u8>> {
    let msgpacked = rmp_serde::to_vec_named(data)?;
    Ok(compress_zstd(&msgpacked))
}

/// 旧版 Section 解包：`zstd -> msgpack -> Value`
pub fn unpack_section_old_style(compressed: &[u8]) -> anyhow::Result<Value> {
    let msgpacked = decompress_zstd(compressed)?;
    let val: Value = rmp_serde::from_slice(&msgpacked)?;
    Ok(val)
}

pub fn find_gmp_file(map_path: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(map_path).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("gmp") {
            return Some(p);
        }
    }
    None
}

/// 保存/更新地图目录下的 `.gmp` 文件（整合 `editor_table/`, `ui/`, `uslanguage.json`, `zhlanguage.json`）
pub fn save_gmp(map_path: &Path, options: GmpSaveOptions) -> anyhow::Result<GmpSaveResult> {
    let gmp_file = match find_gmp_file(map_path) {
        Some(f) => f,
        None => {
            return Ok(GmpSaveResult {
                success: false,
                message: format!("在 {} 下未找到 .gmp 文件", map_path.display()),
                sections_replaced: Vec::new(),
                ui_section_size: None,
                output_size: 0,
            })
        }
    };

    let mut parser = GmpParser::parse_file(&gmp_file)?;
    let section_idx_map = get_prefab_section_index_map();
    let mut sections_replaced = Vec::new();
    let mut ui_section_size = None;

    // 1. 物编 Sections (NewStyle)
    if options.update_prefabs {
        let editor_table_dir = map_path.join("editor_table");
        for &(sec_name, folder_name) in PREFAB_SECTIONS {
            let folder_path = editor_table_dir.join(folder_name);
            let dict = load_json_folder_as_dict(&folder_path);
            let packed = pack_section_new_style(&dict)?;
            if let Some(&sec_idx) = section_idx_map.get(sec_name) {
                parser.sections.insert(
                    sec_idx,
                    SectionData {
                        index: sec_idx,
                        body: packed,
                        is_old_section: false,
                        bin_table: None,
                    },
                );
                sections_replaced.push(sec_name.to_string());
            }
        }
    }

    // 2. UI Section (ID = 10, OldStyle)
    if options.update_ui {
        let ui_data = build_ui_data_dict(map_path);
        let packed = pack_section_old_style(&ui_data)?;
        ui_section_size = Some(packed.len());
        parser.sections.insert(
            UI_SECTION_ID,
            SectionData {
                index: UI_SECTION_ID,
                body: packed,
                is_old_section: true,
                bin_table: None,
            },
        );
    }

    // 3. USLanguage Section (ID = 49, OldStyle)
    if options.update_us_language {
        if let Some(packed) = build_language_section(map_path, "uslanguage.json")? {
            parser.sections.insert(
                US_LANGUAGE_SECTION_ID,
                SectionData {
                    index: US_LANGUAGE_SECTION_ID,
                    body: packed,
                    is_old_section: true,
                    bin_table: None,
                },
            );
        }
    }

    // 4. ZHLanguage Section (ID = 50, OldStyle)
    if options.update_zh_language {
        if let Some(packed) = build_language_section(map_path, "zhlanguage.json")? {
            parser.sections.insert(
                ZH_LANGUAGE_SECTION_ID,
                SectionData {
                    index: ZH_LANGUAGE_SECTION_ID,
                    body: packed,
                    is_old_section: true,
                    bin_table: None,
                },
            );
        }
    }

    let new_bytes = parser.build_bytes();
    let out_file = options.output_path.unwrap_or_else(|| gmp_file.clone());
    if out_file == gmp_file {
        let bak_file = gmp_file.with_extension("gmp.bak");
        let _ = fs::copy(&gmp_file, &bak_file);
    }
    fs::write(&out_file, &new_bytes)?;

    Ok(GmpSaveResult {
        success: true,
        message: format!("GMP 重建成功: {}", out_file.display()),
        sections_replaced,
        ui_section_size,
        output_size: new_bytes.len(),
    })
}

fn load_json_folder_as_dict(folder: &Path) -> Value {
    let mut map = Map::new();
    if !folder.is_dir() {
        return Value::Object(map);
    }
    if let Ok(entries) = fs::read_dir(folder) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<Value>(&content) {
                    let key_str = val
                        .get("uid")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| {
                            path.file_stem()
                                .and_then(|s| s.to_str())
                                .map(|s| s.to_string())
                        });
                    if let Some(k) = key_str {
                        map.insert(k, val);
                    }
                }
            }
        }
    }
    Value::Object(map)
}

fn build_ui_data_dict(map_path: &Path) -> Value {
    let mut ui_data_list = Vec::new();
    let mut prefab_data_map = Map::new();
    let mut root_map = Map::new();

    let ui_dir = map_path.join("ui");
    if ui_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&ui_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.file_name().and_then(|s| s.to_str()) == Some("prefab") {
                    if let Ok(p_entries) = fs::read_dir(&path) {
                        for p_entry in p_entries.flatten() {
                            let p_path = p_entry.path();
                            if p_path.extension().and_then(|s| s.to_str()) == Some("json") {
                                if let Ok(text) = fs::read_to_string(&p_path) {
                                    if let Ok(val) = serde_json::from_str::<Value>(&text) {
                                        let plain = convert_tuple_format(&val);
                                        if let Some(k) = plain.get("key").and_then(|v| v.as_str()) {
                                            prefab_data_map.insert(k.to_string(), plain);
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if path.is_file()
                    && path.extension().and_then(|s| s.to_str()) == Some("json")
                {
                    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    if let Ok(text) = fs::read_to_string(&path) {
                        if let Ok(val) = serde_json::from_str::<Value>(&text) {
                            let plain = convert_tuple_format(&val);
                            if fname == "ui_config.json" {
                                if let Some(obj) = plain.as_object() {
                                    for (k, v) in obj {
                                        if k != "ui_data" && k != "prefab_data" {
                                            root_map.insert(k.clone(), v.clone());
                                        }
                                    }
                                }
                            } else {
                                ui_data_list.push(plain);
                            }
                        }
                    }
                }
            }
        }
    }

    root_map.insert("ui_data".to_string(), Value::Array(ui_data_list));
    root_map.insert("prefab_data".to_string(), Value::Object(prefab_data_map));
    Value::Object(root_map)
}

fn build_language_section(map_path: &Path, filename: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let mut lang_path = map_path.join(filename);
    if !lang_path.is_file() {
        // 尝试项目级 language 目录 (`../../language/{filename}`)
        if let Some(proj_dir) = map_path.parent().and_then(|p| p.parent()) {
            let fallback = proj_dir.join("language").join(filename);
            if fallback.is_file() {
                lang_path = fallback;
            }
        }
    }
    if !lang_path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&lang_path)?;
    let val: Value = serde_json::from_str(&text)?;
    let packed = pack_section_old_style(&val)?;
    Ok(Some(packed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_gmp_parse_and_save_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let map_dir = tmp.path();
        fs::create_dir_all(map_dir.join("editor_table/editorunit")).unwrap();
        fs::create_dir_all(map_dir.join("ui")).unwrap();
        fs::write(
            map_dir.join("editor_table/editorunit/134217728.json"),
            r#"{"uid": "134217728", "key": 134217728, "name": "测试单位"}"#,
        )
        .unwrap();
        fs::write(
            map_dir.join("zhlanguage.json"),
            r#"{"12345": "本地化文本"}"#,
        )
        .unwrap();

        // 构造初始空 GMP 文件
        let initial_parser = GmpParser {
            header: GmpHeader {
                uuid: [1u8; 16],
                version: 5,
                pack_size: 2,
                create_stamp: "20261021".to_string(),
                editor_pack_stamp: "20261021".to_string(),
            },
            sections: BTreeMap::new(),
        };
        let gmp_path = map_dir.join("EntryMap.gmp");
        fs::write(&gmp_path, initial_parser.build_bytes()).unwrap();

        // 执行 save_gmp
        let res = save_gmp(map_dir, GmpSaveOptions::default()).unwrap();
        assert!(res.success);
        assert!(gmp_path.with_extension("gmp.bak").is_file());

        // 重新解析验证 Section 数据
        let reparsed = GmpParser::parse_file(&gmp_path).unwrap();
        let idx_map = get_prefab_section_index_map();
        let unit_idx = idx_map["editor_unit"];
        let unit_sec = &reparsed.sections[&unit_idx];
        let unpacked_unit = unpack_section_new_style(&unit_sec.body).unwrap();
        assert_eq!(unpacked_unit["134217728"]["name"], json!("测试单位"));

        let zh_sec = &reparsed.sections[&ZH_LANGUAGE_SECTION_ID];
        let unpacked_zh = unpack_section_old_style(&zh_sec.body).unwrap();
        assert_eq!(unpacked_zh["12345"], json!("本地化文本"));
    }
}
