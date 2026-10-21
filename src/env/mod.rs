use encoding_rs::GBK;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Y3 编辑器环境信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorEnv {
    /// Editor.exe 完整路径
    pub editor_exe_path: PathBuf,
    /// Game_x64h.exe 完整路径 (`../Engine/Binaries/Win64/Game_x64h.exe`)
    pub game_exe_path: PathBuf,
    /// 编辑器版本 (`"1.0"` 或 `"2.0"`)
    pub version: String,
    /// 编辑器本地配置的用户昵称 (`UserDefault.json` 中的 `nickname`)
    pub nickname: Option<String>,
    /// 本地补丁版本号 (`editor_patchmd5_windows64_normal.txt`)
    pub patch_version: Option<String>,
}

impl EditorEnv {
    /// 自动探测或验证指定的 `Editor.exe` 路径
    pub fn detect(custom_editor_path: Option<&Path>) -> Option<Self> {
        // 1. 优先使用显式传入的路径
        if let Some(p) = custom_editor_path {
            if let Some(env) = Self::from_editor_exe(p.to_path_buf()) {
                return Some(env);
            }
        }
        // 2. 检查持久化的本地配置文件 (%APPDATA%/y3-helper/config.json)
        if let Some(saved) = Self::load_saved_editor_path() {
            if let Some(env) = Self::from_editor_exe(saved) {
                return Some(env);
            }
        }
        // 3. 检查环境变量 Y3_EDITOR_PATH
        if let Ok(env_path) = std::env::var("Y3_EDITOR_PATH") {
            if !env_path.trim().is_empty() {
                if let Some(env) = Self::from_editor_exe(PathBuf::from(env_path.trim())) {
                    return Some(env);
                }
            }
        }
        // 4. 查询 Windows 注册表 (支持 HKLM / HKCU / HKCR 多位置)
        if let Some(reg_path) = Self::search_editor_from_registry() {
            if let Some(env) = Self::from_editor_exe(reg_path) {
                return Some(env);
            }
        }
        None
    }

    fn config_file_path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join("y3-helper").join("config.json")
    }

    pub fn save_editor_path_config(path: &Path) {
        let cfg_path = Self::config_file_path();
        if let Some(parent) = cfg_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let data = serde_json::json!({
            "editor_exe_path": path.to_string_lossy()
        });
        let _ = fs::write(cfg_path, serde_json::to_string_pretty(&data).unwrap_or_default());
    }

    fn load_saved_editor_path() -> Option<PathBuf> {
        let cfg_path = Self::config_file_path();
        let text = fs::read_to_string(cfg_path).ok()?;
        let val: Value = serde_json::from_str(&text).ok()?;
        let s = val.get("editor_exe_path")?.as_str()?.trim();
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    }

    /// 从注册表多路径查询 `Editor.exe` 路径
    pub fn search_editor_from_registry() -> Option<PathBuf> {
        let reg_keys = [
            r"HKLM\SOFTWARE\Classes\y3editor",
            r"HKCU\SOFTWARE\Classes\y3editor",
            r"HKCR\y3editor",
            r"HKLM\SOFTWARE\Classes\y3editor\shell\open\command",
            r"HKCU\SOFTWARE\Classes\y3editor\shell\open\command",
        ];
        let re = Regex::new(r#"([A-Za-z]:\\[^"\r\n]*?Editor\.exe)"#).ok()?;
        #[cfg(target_os = "windows")]
        use std::os::windows::process::CommandExt;
        for key in reg_keys {
            let mut cmd = Command::new("reg.exe");
            cmd.args(["query", key, "/s"]);
            #[cfg(target_os = "windows")]
            cmd.creation_flags(0x08000000);
            if let Ok(output) = cmd.output()
            {
                let (decoded, _, _) = GBK.decode(&output.stdout);
                if let Some(caps) = re.captures(&decoded) {
                    if let Some(matched) = caps.get(1) {
                        let p = PathBuf::from(matched.as_str().trim());
                        if p.is_file() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        None
    }

    /// 根据 `Editor.exe`、`Game_x64h.exe` 或安装目录构造 `EditorEnv`
    pub fn from_editor_exe(input_path: PathBuf) -> Option<Self> {
        // 支持传入目录、Game_x64h.exe 或 Editor.exe
        let mut editor_exe_path = input_path.clone();
        if input_path.is_dir() {
            if input_path.join("Editor.exe").is_file() {
                editor_exe_path = input_path.join("Editor.exe");
            } else if input_path.join("game").join("Editor.exe").is_file() {
                editor_exe_path = input_path.join("game").join("Editor.exe");
            }
        } else if input_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.eq_ignore_ascii_case("Game_x64h.exe"))
            .unwrap_or(false)
        {
            // 若用户直接选择了 Engine/Binaries/Win64/Game_x64h.exe，向上反推 Editor.exe
            if let Some(game_dir) = input_path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
            {
                if game_dir.join("Editor.exe").is_file() {
                    editor_exe_path = game_dir.join("Editor.exe");
                } else if game_dir.join("game").join("Editor.exe").is_file() {
                    editor_exe_path = game_dir.join("game").join("Editor.exe");
                }
            }
        }

        if !editor_exe_path.is_file() {
            return None;
        }
        let editor_dir = editor_exe_path.parent()?;
        let parent_dir = editor_dir.parent().unwrap_or(editor_dir);

        // 在 VSCode Uri.joinPath(editorUri, "../Engine/...") 中，editorUri 是文件 Editor.exe，
        // 因此 "../" 指向的是 editor_dir (即 game 目录)；同时兼容 parent_dir 结构
        let candidate_in_editor_dir = editor_dir
            .join("Engine")
            .join("Binaries")
            .join("Win64")
            .join("Game_x64h.exe");
        let candidate_in_parent_dir = parent_dir
            .join("Engine")
            .join("Binaries")
            .join("Win64")
            .join("Game_x64h.exe");

        let game_exe_path = if candidate_in_editor_dir.is_file() {
            candidate_in_editor_dir
        } else if candidate_in_parent_dir.is_file() {
            candidate_in_parent_dir
        } else {
            candidate_in_editor_dir
        };

        let engine_base_dir = if editor_dir.join("Engine").is_dir() {
            editor_dir
        } else {
            parent_dir
        };

        let version = parent_dir
            .file_name()
            .and_then(|s| s.to_str())
            .map(|name| if name == "1.0" { "1.0" } else { "2.0" })
            .unwrap_or("2.0")
            .to_string();

        let user_default_path = engine_base_dir
            .join("Engine")
            .join("Binaries")
            .join("Win64")
            .join("UserDefault.json");
        let nickname = Self::read_nickname(&user_default_path);

        let patch_md5_path = engine_base_dir.join("editor_patchmd5_windows64_normal.txt");
        let patch_version = Self::read_patch_version(&patch_md5_path);

        Some(Self {
            editor_exe_path,
            game_exe_path,
            version,
            nickname,
            patch_version,
        })
    }

    fn read_nickname(user_default_path: &Path) -> Option<String> {
        let text = fs::read_to_string(user_default_path).ok()?;
        let json: Value = serde_json::from_str(&text).ok()?;
        let obj = json.as_object()?;
        if let Some(nick) = obj
            .get("nickname")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            return Some(nick.to_string());
        }
        for (_, v) in obj {
            if let Some(sub) = v.as_object() {
                if let Some(nick) = sub
                    .get("nickname")
                    .and_then(|n| n.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                {
                    return Some(nick.to_string());
                }
            }
        }
        None
    }

    fn read_patch_version(patch_md5_path: &Path) -> Option<String> {
        let text = fs::read_to_string(patch_md5_path).ok()?;
        let first_line = text.lines().next()?;
        let re = Regex::new(r#""version":\s*"([^"]+)""#).ok()?;
        let caps = re.captures(first_line)?;
        Some(caps.get(1)?.as_str().to_string())
    }
}

/// 单个子地图（`maps/<map_name>`）信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapEntry {
    /// 地图唯一数字 ID（来自 `header.map`）
    pub id: u64,
    /// 地图目录名（如 `"EntryMap"`）
    pub name: String,
    /// 地图描述名（来自 `desc.json`）
    pub description: String,
    /// 地图根目录（`{project}/maps/{name}`）
    pub map_path: PathBuf,
    /// 脚本目录（`{map_path}/script`）
    pub script_path: PathBuf,
    /// 是否为主地图（EntryMap）
    pub is_entry_map: bool,
}

/// 单个 Y3 地图项目上下文（支持项目下包含多张子地图）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContext {
    /// 项目短 ID（基于规范化路径生成，便于 API/CLI 快速引用）
    pub project_id: String,
    /// 项目根目录（含 `header.project`）
    pub project_path: PathBuf,
    /// `header.project` 文件路径
    pub header_project_path: PathBuf,
    /// `maps` 目录路径
    pub maps_dir_path: PathBuf,
    /// `global_script` 目录路径
    pub global_script_path: PathBuf,
    /// 主地图 ID
    pub entry_map_id: u64,
    /// 主地图名称
    pub entry_map_name: String,
    /// 是否使用主关卡触发器和物编（来自 `setting.json`）
    pub use_main_level_trigger_and_object: bool,
    /// 是否使用主关卡 UI（来自 `setting.json`）
    pub use_main_level_ui: bool,
    /// 项目下的所有子地图（键为地图目录名 `name`）
    pub maps: BTreeMap<String, MapEntry>,
    /// 当前默认选中的地图名
    pub current_map_name: String,
}

impl ProjectContext {
    /// 从任意路径（项目根目录、子地图目录、`script` 目录或内部文件路径）向上最多 20 层搜索 `header.project` 并加载项目
    pub fn load_from_path(start_path: &Path) -> anyhow::Result<Self> {
        let abs_start = if start_path.is_absolute() {
            start_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(start_path)
        };
        let normalized_start = normalize_path(&abs_start);

        let (project_path, initial_map_hint) = Self::find_project_root(&normalized_start)?;
        let header_project_path = project_path.join("header.project");
        let maps_dir_path = project_path.join("maps");
        let global_script_path = project_path.join("global_script");

        let entry_map_id = Self::parse_entry_map_id(&header_project_path).unwrap_or(0);
        let (use_main_level_trigger_and_object, use_main_level_ui) =
            Self::parse_project_settings(&project_path);

        let maps = Self::scan_maps(&maps_dir_path, entry_map_id)?;
        if maps.is_empty() {
            anyhow::bail!(
                "未在项目目录 {} 的 maps/ 下找到任何有效子地图",
                project_path.display()
            );
        }

        let entry_map_name = maps
            .values()
            .find(|m| m.is_entry_map)
            .or_else(|| maps.get("EntryMap"))
            .or_else(|| maps.values().next())
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "EntryMap".to_string());

        let current_map_name = initial_map_hint
            .filter(|hint| maps.contains_key(hint))
            .unwrap_or_else(|| entry_map_name.clone());

        let project_id = compute_project_id(&project_path);

        Ok(Self {
            project_id,
            project_path,
            header_project_path,
            maps_dir_path,
            global_script_path,
            entry_map_id,
            entry_map_name,
            use_main_level_trigger_and_object,
            use_main_level_ui,
            maps,
            current_map_name,
        })
    }

    /// 重新扫描项目配置和所有子地图
    pub fn reload(&mut self) -> anyhow::Result<()> {
        self.entry_map_id = Self::parse_entry_map_id(&self.header_project_path).unwrap_or(0);
        let (use_trig, use_ui) = Self::parse_project_settings(&self.project_path);
        self.use_main_level_trigger_and_object = use_trig;
        self.use_main_level_ui = use_ui;
        self.maps = Self::scan_maps(&self.maps_dir_path, self.entry_map_id)?;
        if !self.maps.contains_key(&self.current_map_name) {
            if let Some(entry) = self
                .maps
                .values()
                .find(|m| m.is_entry_map)
                .or_else(|| self.maps.values().next())
            {
                self.current_map_name = entry.name.clone();
            }
        }
        Ok(())
    }

    /// 切换该项目当前的默认地图
    pub fn switch_map(&mut self, map_name: &str) -> anyhow::Result<MapEntry> {
        if let Some(m) = self.maps.get(map_name) {
            self.current_map_name = m.name.clone();
            return Ok(m.clone());
        }
        if let Some(m) = self.maps.values().find(|m| m.description == map_name) {
            self.current_map_name = m.name.clone();
            return Ok(m.clone());
        }
        anyhow::bail!(
            "在项目 {} 中未找到子地图 '{}'（可用地图：{:?}）",
            self.project_path.display(),
            map_name,
            self.maps.keys().collect::<Vec<_>>()
        )
    }

    /// 获取目标子地图（若 `map_name` 为 `None` 则返回 `current_map_name` 对应的子地图）
    pub fn resolve_map(&self, map_name: Option<&str>) -> anyhow::Result<&MapEntry> {
        if let Some(name) = map_name.filter(|s| !s.trim().is_empty()) {
            if let Some(m) = self.maps.get(name) {
                return Ok(m);
            }
            if let Some(m) = self.maps.values().find(|m| m.description == name) {
                return Ok(m);
            }
            anyhow::bail!(
                "在项目 {} 中未找到子地图 '{}'（可用地图：{:?}）",
                self.project_path.display(),
                name,
                self.maps.keys().collect::<Vec<_>>()
            );
        }
        self.maps
            .get(&self.current_map_name)
            .or_else(|| self.maps.get(&self.entry_map_name))
            .or_else(|| self.maps.values().next())
            .ok_or_else(|| anyhow::anyhow!("项目中没有任何有效子地图"))
    }

    /// 获取主地图（EntryMap）
    pub fn entry_map(&self) -> anyhow::Result<&MapEntry> {
        self.maps
            .get(&self.entry_map_name)
            .or_else(|| self.maps.values().find(|m| m.is_entry_map))
            .or_else(|| self.maps.values().next())
            .ok_or_else(|| anyhow::anyhow!("未找到 EntryMap"))
    }

    /// 获取指定子地图实际读取触发器与物编的地图（若 `use_main_level_trigger_and_object` 为 true 则返回 `entry_map`）
    pub fn trigger_map_for(&self, map_name: Option<&str>) -> anyhow::Result<&MapEntry> {
        if self.use_main_level_trigger_and_object {
            self.entry_map()
        } else {
            self.resolve_map(map_name)
        }
    }

    /// 获取指定子地图实际读取 UI 的地图（若 `use_main_level_ui` 为 true 则返回 `entry_map`）
    pub fn ui_map_for(&self, map_name: Option<&str>) -> anyhow::Result<&MapEntry> {
        if self.use_main_level_ui {
            self.entry_map()
        } else {
            self.resolve_map(map_name)
        }
    }

    /// 获取项目级语言文件目录（`{project_path}/language`），若不存在则回退到物编地图根目录
    pub fn language_dir_for(&self, map_name: Option<&str>) -> anyhow::Result<PathBuf> {
        let proj_lang = self.project_path.join("language");
        if proj_lang.is_dir() {
            Ok(proj_lang)
        } else {
            Ok(self.trigger_map_for(map_name)?.map_path.clone())
        }
    }

    fn find_project_root(start: &Path) -> anyhow::Result<(PathBuf, Option<String>)> {
        let mut current = if start.is_file() {
            start.parent().unwrap_or(start).to_path_buf()
        } else {
            start.to_path_buf()
        };

        let mut map_hint: Option<String> = None;

        for _ in 0..=20 {
            if current.join("header.project").is_file() {
                return Ok((current, map_hint));
            }
            if current.join("header.map").is_file() {
                if let Some(name) = current.file_name().and_then(|n| n.to_str()) {
                    map_hint = Some(name.to_string());
                }
            }
            match current.parent() {
                Some(p) if p != current => current = p.to_path_buf(),
                _ => break,
            }
        }

        anyhow::bail!(
            "无法在路径 {} 及其向上 20 层父目录中找到 header.project",
            start.display()
        )
    }

    fn parse_entry_map_id(header_project_path: &Path) -> Option<u64> {
        let text = fs::read_to_string(header_project_path).ok()?;
        let val: Value = serde_json::from_str(&text).ok()?;
        val.get("entry_map")?
            .get("id")?
            .as_u64()
            .or_else(|| {
                // 兼容超大整数或正则提取
                let re = Regex::new(r#""entry_map"\s*:\s*\{[^}]*"id"\s*:\s*(\d+)"#).ok()?;
                let caps = re.captures(&text)?;
                caps.get(1)?.as_str().parse::<u64>().ok()
            })
    }

    fn parse_project_settings(project_path: &Path) -> (bool, bool) {
        let setting_path = project_path.join("setting.json");
        if let Ok(text) = fs::read_to_string(setting_path) {
            if let Ok(val) = serde_json::from_str::<Value>(&text) {
                let use_trig = val
                    .get("use_main_level_trigger_and_object")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let use_ui = val
                    .get("use_main_level_ui")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                return (use_trig, use_ui);
            }
        }
        (false, false)
    }

    fn scan_maps(maps_dir: &Path, entry_map_id: u64) -> anyhow::Result<BTreeMap<String, MapEntry>> {
        let mut result = BTreeMap::new();
        if !maps_dir.is_dir() {
            return Ok(result);
        }
        let id_re = Regex::new(r#""id"\s*:\s*(\d+)"#)?;

        for entry in fs::read_dir(maps_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let header_map_path = path.join("header.map");
            let id = if header_map_path.is_file() {
                let text = fs::read_to_string(&header_map_path).unwrap_or_default();
                id_re
                    .captures(&text)
                    .and_then(|c| c.get(1))
                    .and_then(|m| m.as_str().parse::<u64>().ok())
                    .unwrap_or(0)
            } else {
                0
            };

            let desc_path = path.join("desc.json");
            let description = if desc_path.is_file() {
                let text = fs::read_to_string(&desc_path).unwrap_or_default();
                serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| {
                        v.get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.trim_start_matches('(').trim_end_matches(')').to_string())
                    })
                    .unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };

            let script_path = path.join("script");
            // 确保 script/.log 目录存在（与原版 env.ts 一致）
            let log_dir = script_path.join(".log");
            let _ = fs::create_dir_all(&log_dir);

            let is_entry_map = (entry_map_id != 0 && id == entry_map_id)
                || (entry_map_id == 0 && name == "EntryMap");

            result.insert(
                name.clone(),
                MapEntry {
                    id,
                    name,
                    description,
                    map_path: path,
                    script_path,
                    is_entry_map,
                },
            );
        }
        Ok(result)
    }
}

/// 规范化路径（消除 `\\?\` 前缀、统一路径分隔符）
pub fn normalize_path(path: &Path) -> PathBuf {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = resolved.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        resolved
    }
}

/// 生成稳定的短 `project_id`（`<目录名>-<8位哈希>`）
pub fn compute_project_id(project_path: &Path) -> String {
    let norm = normalize_path(project_path)
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(norm.as_bytes());
    let hex_hash = hex::encode(hasher.finalize());
    let folder = project_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    format!("{}-{}", folder, &hex_hash[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_context_multi_maps() {
        let tmp = tempfile::tempdir().unwrap();
        let proj_dir = tmp.path().join("MyY3Project");
        fs::create_dir_all(proj_dir.join("maps/EntryMap/script")).unwrap();
        fs::create_dir_all(proj_dir.join("maps/DungeonMap/script")).unwrap();

        fs::write(
            proj_dir.join("header.project"),
            r#"{"entry_map": {"id": 1001}}"#,
        )
        .unwrap();
        fs::write(
            proj_dir.join("setting.json"),
            r#"{"use_main_level_trigger_and_object": true, "use_main_level_ui": false}"#,
        )
        .unwrap();
        fs::write(proj_dir.join("maps/EntryMap/header.map"), r#"{"id": 1001}"#).unwrap();
        fs::write(
            proj_dir.join("maps/EntryMap/desc.json"),
            r#"{"name": "(主地图)"}"#,
        )
        .unwrap();
        fs::write(
            proj_dir.join("maps/DungeonMap/header.map"),
            r#"{"id": 1002}"#,
        )
        .unwrap();
        fs::write(
            proj_dir.join("maps/DungeonMap/desc.json"),
            r#"{"name": "(副本地图)"}"#,
        )
        .unwrap();

        let mut ctx = ProjectContext::load_from_path(&proj_dir.join("maps/DungeonMap")).unwrap();
        assert_eq!(ctx.maps.len(), 2);
        assert_eq!(ctx.entry_map_name, "EntryMap");
        assert_eq!(ctx.current_map_name, "DungeonMap");

        // 因为 use_main_level_trigger_and_object = true，所以 DungeonMap 的 trigger_map 是 EntryMap
        assert_eq!(ctx.trigger_map_for(Some("DungeonMap")).unwrap().name, "EntryMap");
        // 因为 use_main_level_ui = false，所以 DungeonMap 的 ui_map 是 DungeonMap
        assert_eq!(ctx.ui_map_for(Some("DungeonMap")).unwrap().name, "DungeonMap");

        // 切换地图测试
        ctx.switch_map("主地图").unwrap();
        assert_eq!(ctx.current_map_name, "EntryMap");
    }
}
