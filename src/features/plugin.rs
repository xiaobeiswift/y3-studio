use crate::env::ProjectContext;
use crate::meta_builder::MetaBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TPL_PLUGIN_EXAMPLE: &str =
    include_str!("../../y3-helper-vscode/template/plugin/1.js");
const TPL_PLUGIN_TABLE: &str =
    include_str!("../../y3-helper-vscode/template/plugin/2.js");
const TPL_PLUGIN_EXCEL: &str =
    include_str!("../../y3-helper-vscode/template/plugin/3.js");
const TPL_PLUGIN_SAVE: &str =
    include_str!("../../y3-helper-vscode/template/plugin/4.js");
const TPL_Y3_HELPER_DTS: &str = include_str!("../../y3-helper-vscode/template/plugin/y3-helper.d.ts");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub path: PathBuf,
    pub exported_functions: Vec<String>,
}

/// 插件管理器（对应原版 `src/plugin/`，剥离 VS Code CodeLens 后提供扫描、初始化与运行能力）
pub struct PluginManager;

impl PluginManager {
    /// 初始化指定地图的 `script/y3-helper/plugin/` 模板文件
    pub fn init_plugins(project: &ProjectContext, map_name: Option<&str>) -> anyhow::Result<PathBuf> {
        let map = project.resolve_map(map_name)?;
        let plugin_dir = map.script_path.join("y3-helper/plugin");
        fs::create_dir_all(&plugin_dir)?;

        let templates = [
            ("1-使用插件.js", TPL_PLUGIN_EXAMPLE),
            ("2-读取与修改物编.js", TPL_PLUGIN_TABLE),
            ("3-读取Excel表格.js", TPL_PLUGIN_EXCEL),
            ("6-保存地图时执行.js", TPL_PLUGIN_SAVE),
            ("y3-helper.d.ts", TPL_Y3_HELPER_DTS),
        ];
        for (fname, content) in templates {
            let p = plugin_dir.join(fname);
            if !p.exists() {
                fs::write(p, content)?;
            }
        }
        let _ = MetaBuilder::build_for_map(project, Some(&map.name));
        Ok(plugin_dir)
    }

    /// 扫描指定地图（及 `global_script`）下的所有 `.js` 插件及其导出函数
    pub fn list_plugins(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Vec<PluginInfo>> {
        let map = project.resolve_map(map_name)?;
        let mut dirs = vec![map.script_path.join("y3-helper/plugin")];
        if project.global_script_path.is_dir() {
            dirs.push(project.global_script_path.join("y3-helper/plugin"));
        }

        let func_re = Regex::new(
            r"(?:export\s+(?:async\s+)?function\s+([a-zA-Z0-9_\p{Han}]+)|exports\.([a-zA-Z0-9_\p{Han}]+)\s*=)",
        )?;
        let mut list = Vec::new();

        for d in dirs {
            if !d.is_dir() {
                continue;
            }
            for entry in fs::read_dir(d)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("js") {
                    continue;
                }
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("plugin")
                    .to_string();
                let content = fs::read_to_string(&path).unwrap_or_default();
                let mut funcs = Vec::new();
                for caps in func_re.captures_iter(&content) {
                    if let Some(m) = caps.get(1).or_else(|| caps.get(2)) {
                        funcs.push(m.as_str().to_string());
                    }
                }
                list.push(PluginInfo {
                    name,
                    path,
                    exported_functions: funcs,
                });
            }
        }
        Ok(list)
    }

    /// 运行指定插件函数（例如 `"onSave"`, `"onGame"`, `"onEditor"` 或自定义导出函数）
    pub fn run_plugin_function(
        project: &ProjectContext,
        map_name: Option<&str>,
        plugin_name_or_all: Option<&str>,
        func_name: &str,
    ) -> anyhow::Result<Vec<String>> {
        let plugins = Self::list_plugins(project, map_name)?;
        let mut outputs = Vec::new();
        for p in plugins {
            if let Some(target) = plugin_name_or_all {
                if p.name != target && p.path.to_string_lossy() != target {
                    continue;
                }
            }
            if !p.exported_functions.iter().any(|f| f == func_name) {
                continue;
            }
            let out = Self::execute_js_file(&p.path, func_name, &project.project_path)?;
            outputs.push(format!("[{}:{}] {}", p.name, func_name, out));
        }
        Ok(outputs)
    }

    fn execute_js_file(js_path: &Path, func_name: &str, project_root: &Path) -> anyhow::Result<String> {
        let js_code = fs::read_to_string(js_path)?;
        // 将 ES Module `export async function` / `export function` 转换为 CommonJS 以便 Node 直接执行
        let cjs_code = js_code
            .replace("export async function ", "exports.")
            .replace("export function ", "exports.");
        let _ = cjs_code;
        let wrapper = format!(
            r#"
            const fs = require('fs');
            const path = require('path');
            let code = fs.readFileSync({js_path:?}, 'utf-8');
            code = code.replace(/export\s+async\s+function\s+([a-zA-Z0-9_\u4e00-\u9fa5]+)/g, 'exports.$1 = async function $1');
            code = code.replace(/export\s+function\s+([a-zA-Z0-9_\u4e00-\u9fa5]+)/g, 'exports.$1 = function $1');
            code = code.replace(/import\s+\*\s+as\s+y3\s+from\s+['"]y3-helper['"];?/g, '');
            const y3 = {{
                env: {{ projectUri: {{ fsPath: {proj_root:?} }} }},
                log: console,
                print: console.log,
            }};
            const mod = {{ exports: {{}} }};
            const fn = new Function('exports', 'require', 'module', 'y3', code);
            fn(mod.exports, require, mod, y3);
            Promise.resolve(mod.exports[{func_name:?}] && mod.exports[{func_name:?}]()).then(() => process.exit(0)).catch(e => {{ console.error(e); process.exit(1); }});
            "#,
            js_path = js_path.to_string_lossy(),
            proj_root = project_root.to_string_lossy(),
            func_name = func_name
        );

        let output = Command::new("node")
            .args(["-e", &wrapper])
            .current_dir(project_root)
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            anyhow::bail!("插件执行失败: {}", stderr);
        }
        Ok(stdout)
    }
}
