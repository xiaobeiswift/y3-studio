use crate::env::ProjectContext;
use crate::meta_builder::MetaBuilder;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// 全局脚本管理器（对应原版 `src/globalScript.ts`）
pub struct GlobalScriptManager;

impl GlobalScriptManager {
    pub fn enable(project: &ProjectContext) -> anyhow::Result<Vec<String>> {
        let entry_map = project.entry_map()?;
        let src_y3 = entry_map.script_path.join("y3");
        let dst_y3 = project.global_script_path.join("y3");

        fs::create_dir_all(&project.global_script_path)?;

        // 1. 复制 EntryMap/script/y3 到 global_script/y3
        if src_y3.is_dir() && !dst_y3.exists() {
            Self::copy_dir_all(&src_y3, &dst_y3)?;
        }

        let mut updated_files = Vec::new();

        // 2. 更新所有子地图的 script/.luarc.json
        for map in project.maps.values() {
            let luarc_path = map.script_path.join(".luarc.json");
            if luarc_path.is_file() {
                if let Ok(text) = fs::read_to_string(&luarc_path) {
                    if let Ok(mut val) = serde_json::from_str::<Value>(&text) {
                        Self::inject_global_script_paths(&mut val);
                        let formatted = serde_json::to_string_pretty(&val)?;
                        fs::write(&luarc_path, formatted)?;
                        updated_files.push(luarc_path.to_string_lossy().to_string());
                    }
                }
            }
        }

        // 3. 生成 global_script/.luarc.json
        let global_luarc_path = project.global_script_path.join(".luarc.json");
        if !global_luarc_path.exists() {
            let entry_luarc = entry_map.script_path.join(".luarc.json");
            if entry_luarc.is_file() {
                if let Ok(text) = fs::read_to_string(&entry_luarc) {
                    if let Ok(mut val) = serde_json::from_str::<Value>(&text) {
                        if let Some(obj) = val.as_object_mut() {
                            if let Some(Value::Array(libs)) = obj.get_mut("workspace.library") {
                                libs.retain(|x| {
                                    x.as_str()
                                        .map(|s| !s.starts_with("../../../global_script"))
                                        .unwrap_or(true)
                                });
                            }
                        }
                        fs::write(&global_luarc_path, serde_json::to_string_pretty(&val)?)?;
                        updated_files.push(global_luarc_path.to_string_lossy().to_string());
                    }
                }
            } else {
                let default_luarc = json!({
                    "runtime.version": "Lua 5.4",
                    "runtime.path": ["?.lua", "?/init.lua"],
                    "workspace.library": ["./y3"]
                });
                fs::write(
                    &global_luarc_path,
                    serde_json::to_string_pretty(&default_luarc)?,
                )?;
                updated_files.push(global_luarc_path.to_string_lossy().to_string());
            }
        }

        // 4. 触发 Meta 重新生成（将同步写入 global_script/y3-helper/meta/）
        let _ = MetaBuilder::build_all_maps(project);

        Ok(updated_files)
    }

    fn inject_global_script_paths(val: &mut Value) {
        if let Some(obj) = val.as_object_mut() {
            let rt_paths = obj
                .entry("runtime.path")
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(arr) = rt_paths {
                for p in [
                    "../../../global_script/?.lua",
                    "../../../global_script/?/init.lua",
                ] {
                    if !arr.iter().any(|x| x.as_str() == Some(p)) {
                        arr.push(Value::String(p.to_string()));
                    }
                }
            }

            let libs = obj
                .entry("workspace.library")
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(arr) = libs {
                let g = "../../../global_script";
                if !arr.iter().any(|x| x.as_str() == Some(g)) {
                    arr.push(Value::String(g.to_string()));
                }
            }
        }
    }

    fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
        for entry in WalkDir::new(src).into_iter().flatten() {
            if let Ok(rel) = entry.path().strip_prefix(src) {
                let target = dst.join(rel);
                if entry.file_type().is_dir() {
                    fs::create_dir_all(&target)?;
                } else if entry.file_type().is_file() {
                    if let Some(p) = target.parent() {
                        fs::create_dir_all(p)?;
                    }
                    fs::copy(entry.path(), &target)?;
                }
            }
        }
        Ok(())
    }
}
