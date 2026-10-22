use crate::custom_define::CustomDefine;
use crate::eca_compiler::EcaCompiler;
use crate::editor_table::excel::{apply_excel_rule, ExcelImportRule};
use crate::editor_table::table::{EditorTable, TableType};
use crate::features::cloud_script::CloudScriptManager;
use crate::features::global_script::GlobalScriptManager;
use crate::features::lua_doc::LuaDocGenerator;
use crate::features::network_server::NetworkTestServer;
use crate::features::plugin::PluginManager;
use crate::features::project_init::{ProjectInitializer, RepoSource};
use crate::gmp::{save_gmp, GmpSaveOptions};
use crate::launcher::{GameLauncher, LaunchGameOptions};
use crate::meta_builder::MetaBuilder;
use crate::workspace::MultiProjectManager;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use parking_lot::RwLock;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

pub const DEFAULT_MCP_HTTP_PORT: u16 = 8766;
const DESKTOP_UI_HTML: &str = include_str!("ui.html");

pub const MCP_INSTRUCTIONS: &str = "Y3-Helper Standalone Rust Server: supports multi-project and multi-map concurrent operations on the same machine. Every tool accepts optional `projectPath` (or `projectId`) and `mapName` arguments to target a specific project and map without interfering with other running projects.";

/// 核心工具分发器（同时服务于 Streamable HTTP MCP、Stdio MCP 与 REST API）
pub struct ToolDispatcher {
    pub manager: Arc<MultiProjectManager>,
}

impl ToolDispatcher {
    pub fn new(manager: Arc<MultiProjectManager>) -> Arc<Self> {
        Arc::new(Self { manager })
    }

    fn extract_project_selector(args: &Value) -> Option<&str> {
        args.get("projectPath")
            .or_else(|| args.get("project_path"))
            .or_else(|| args.get("projectId"))
            .or_else(|| args.get("project_id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
    }

    fn extract_map_name(args: &Value) -> Option<&str> {
        args.get("mapName")
            .or_else(|| args.get("map_name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
    }

    /// 执行任意工具并返回 JSON 结果
    pub async fn call_tool(&self, name: &str, args: &Value) -> anyhow::Result<Value> {
        let proj_sel = Self::extract_project_selector(args);
        let map_name = Self::extract_map_name(args);

        match name {
            "set_editor_path" => {
                let path_str = args
                    .get("editorPath")
                    .or_else(|| args.get("editor_path"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("缺少 editorPath 参数"))?;
                let env = self.manager.set_editor_path(Path::new(path_str))?;
                Ok(serde_json::to_value(env)?)
            }
            "set_project_path" | "register_project" => {
                let path_str = proj_sel.ok_or_else(|| {
                    anyhow::anyhow!("缺少 projectPath 参数")
                })?;
                let set_active = args
                    .get("setActive")
                    .or_else(|| args.get("set_active"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let ws = self
                    .manager
                    .register_project(Path::new(path_str), set_active)
                    .await?;
                if let Some(m) = map_name {
                    let _ = ws.context.write().switch_map(m);
                }
                Ok(ws.summary_json(set_active))
            }
            "list_projects" => Ok(self.manager.list_projects_json()),
            "remove_project" => {
                let sel = proj_sel.ok_or_else(|| anyhow::anyhow!("缺少 projectPath 或 projectId"))?;
                let removed = self.manager.remove_project(sel);
                Ok(json!({ "success": removed, "target": sel }))
            }
            "get_helper_status" => Ok(self.manager.list_projects_json()),
            "list_maps" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read();
                let maps: Vec<Value> = ctx
                    .maps
                    .values()
                    .map(|m| {
                        json!({
                            "id": m.id,
                            "name": m.name,
                            "description": m.description,
                            "map_path": m.map_path,
                            "is_entry_map": m.is_entry_map,
                            "is_current_map": m.name == ctx.current_map_name,
                        })
                    })
                    .collect();
                Ok(json!({
                    "project_id": ws.project_id,
                    "current_map_name": ctx.current_map_name,
                    "maps": maps
                }))
            }
            "switch_map" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let target_map = map_name.ok_or_else(|| anyhow::anyhow!("缺少 mapName 参数"))?;
                let entry = ws.context.write().switch_map(target_map)?;
                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "current_map_name": entry.name,
                    "map_id": entry.id,
                    "map_path": entry.map_path
                }))
            }
            "init_project" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let repo_source = RepoSource::parse(
                    args.get("repoSource")
                        .or_else(|| args.get("repo_source"))
                        .and_then(|v| v.as_str()),
                );
                let ed = self.manager.editor_env();
                let ctx = ws.context.read().clone();
                let map_owned = map_name.map(|s| s.to_string());
                let report = tokio::task::spawn_blocking(move || {
                    ProjectInitializer::init_map(&ctx, map_owned.as_deref(), repo_source, ed.as_ref(), true)
                }).await??;
                Ok(serde_json::to_value(report)?)
            }
            "get_y3maker_hub" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let mcp_port = args
                    .get("mcpPort")
                    .or_else(|| args.get("mcp_port"))
                    .and_then(|v| v.as_u64())
                    .map(|p| p as u16)
                    .unwrap_or(8768);
                let ctx = ws.context.read().clone();
                let hub = ProjectInitializer::inspect_hub_status(&ctx, mcp_port);
                Ok(serde_json::to_value(hub)?)
            }
            "sync_y3maker" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let repo_source = args
                    .get("repoSource")
                    .or_else(|| args.get("repo_source"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let mcp_port = args
                    .get("mcpPort")
                    .or_else(|| args.get("mcp_port"))
                    .and_then(|v| v.as_u64())
                    .map(|p| p as u16)
                    .unwrap_or(8768);
                let ctx = ws.context.read().clone();
                tokio::task::spawn_blocking(move || {
                    ProjectInitializer::sync_y3maker_hub(&ctx, repo_source.as_deref(), mcp_port)
                })
                .await?
            }
            "equip_agents" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let targets: Vec<String> = args
                    .get("targets")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let mcp_port = args
                    .get("mcpPort")
                    .or_else(|| args.get("mcp_port"))
                    .and_then(|v| v.as_u64())
                    .map(|p| p as u16)
                    .unwrap_or(8768);
                let ctx = ws.context.read().clone();
                tokio::task::spawn_blocking(move || {
                    ProjectInitializer::equip_agents(&ctx, &targets, mcp_port)
                })
                .await?
            }
            "check_y3maker_update" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let repo_source = args
                    .get("repoSource")
                    .or_else(|| args.get("repo_source"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let ctx = ws.context.read().clone();
                tokio::task::spawn_blocking(move || {
                    ProjectInitializer::check_y3maker_remote_update(&ctx, repo_source.as_deref())
                })
                .await?
            }
            "get_skill_detail" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let skill_id = args
                    .get("skillId")
                    .or_else(|| args.get("skill_id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("缺少 skillId 参数"))?;
                let ctx = ws.context.read().clone();
                ProjectInitializer::get_skill_detail(&ctx, skill_id)
            }
            "launch_game" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let multi_mode = args
                    .get("multiMode")
                    .or_else(|| args.get("multi_mode"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let multi_players: Option<Vec<u32>> = args
                    .get("multiPlayers")
                    .or_else(|| args.get("multi_players"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_u64().map(|n| n as u32))
                            .collect()
                    });
                let tracy = args.get("tracy").and_then(|v| v.as_bool()).unwrap_or(false);
                let opts = LaunchGameOptions {
                    map_name: map_name.map(|s| s.to_string()),
                    multi_mode,
                    multi_players,
                    tracy,
                    lua_args: Default::default(),
                    save_gmp_before_launch: Some(true),
                };
                let ctx = ws.context.read().clone();
                let ed = self.manager.editor_env();
                Ok(ws.session_manager.launch_game(&ctx, ed.as_ref(), opts).await)
            }
            "get_game_status" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                Ok(ws.session_manager.get_game_status())
            }
            "execute_lua" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let code = args
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let slot = args
                    .get("clientSlot")
                    .or_else(|| args.get("client_slot"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                Ok(ws.session_manager.execute_lua(code, slot).await)
            }
            "reload_lua" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let count = ws.console_server.reload_lua_all();
                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "reloaded_clients": count,
                    "message": format!("已向项目 {} 的 {} 个客户端发送 .rd 热重载指令", ws.project_id, count)
                }))
            }
            "quick_restart" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let slot = args
                    .get("clientSlot")
                    .or_else(|| args.get("client_slot"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                Ok(ws.session_manager.quick_restart(slot).await)
            }
            "stop_game" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let slot = args
                    .get("clientSlot")
                    .or_else(|| args.get("client_slot"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                Ok(ws.session_manager.stop_game(slot).await)
            }
            "kill_game" => {
                let kill_all = args
                    .get("killAll")
                    .or_else(|| args.get("kill_all"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if kill_all {
                    let killed = GameLauncher::kill_game(None, &[], true)?;
                    return Ok(json!({
                        "success": true,
                        "killed_pids": killed,
                        "scope": "all_projects"
                    }));
                }
                let ws = self.manager.resolve_project(proj_sel).await?;
                let known = ws.session_manager.known_pids();
                let killed = GameLauncher::kill_game(Some(&ws.project_path), &known, false)?;
                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "killed_pids": killed,
                    "scope": "project_only"
                }))
            }
            "launch_editor" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ed = self
                    .manager
                    .editor_env()
                    .ok_or_else(|| anyhow::anyhow!("未检测到 Y3 编辑器路径"))?;
                let ctx = ws.context.read().clone();
                let pid = GameLauncher::launch_editor(&ctx, &ed)?;
                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "editor_pid": pid
                }))
            }
            "get_logs" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
                let slot = args
                    .get("clientSlot")
                    .or_else(|| args.get("client_slot"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                let script_dir = {
                    let ctx = ws.context.read();
                    ctx.trigger_map_for(map_name)
                        .ok()
                        .map(|m| m.script_path.clone())
                };
                Ok(ws.session_manager.get_logs(limit, slot, script_dir.as_deref()))
            }
            "capture_screenshot" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let slot = args
                    .get("clientSlot")
                    .or_else(|| args.get("client_slot"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                let ed = self.manager.editor_env();
                Ok(ws.session_manager.capture_screenshot(ed.as_ref(), slot).await)
            }
            "read_problems_lua" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                Ok(crate::console::session::GameSessionManager::read_problems_lua(
                    &ctx,
                    args.get("pathGlob"),
                ))
            }
            "get_ui_canvas" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let node_path = args.get("nodePath").and_then(|v| v.as_str());
                let raw_depth = args.get("depth").and_then(|v| v.as_i64());
                let max_depth = match raw_depth {
                    Some(-1) => None,
                    Some(d) if d >= 0 => Some(d as usize),
                    _ => Some(1),
                };
                let pkg = CustomDefine::get_ui_package(&ctx, map_name)?;
                match CustomDefine::format_ui_canvas_tree(&pkg, node_path, max_depth) {
                    Ok(tree) => Ok(json!({
                        "success": true,
                        "project_id": ws.project_id,
                        "canvas": tree
                    })),
                    Err(e) => Ok(json!({
                        "success": false,
                        "project_id": ws.project_id,
                        "error": e.to_string()
                    })),
                }
            }
            "compile_eca" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let report = EcaCompiler::compile_map(&ctx, map_name, None)?;
                Ok(serde_json::to_value(report)?)
            }
            "generate_meta" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                if map_name.is_some() {
                    let rep = MetaBuilder::build_for_map(&ctx, map_name)?;
                    Ok(serde_json::to_value(rep)?)
                } else {
                    let reps = MetaBuilder::build_all_maps(&ctx)?;
                    Ok(serde_json::to_value(reps)?)
                }
            }
            "save_gmp" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let target_map = ctx.trigger_map_for(map_name)?;
                let opts = GmpSaveOptions {
                    update_prefabs: args
                        .get("updatePrefabs")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    update_ui: args
                        .get("updateUI")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    update_us_language: args
                        .get("updateUSLanguage")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    update_zh_language: args
                        .get("updateZHLanguage")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    output_path: None,
                };
                let res = save_gmp(&target_map.map_path, opts)?;
                Ok(serde_json::to_value(res)?)
            }
            "editor_table_list" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let t_str = args
                    .get("tableType")
                    .or_else(|| args.get("table_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unit");
                let tt = TableType::parse(t_str)
                    .ok_or_else(|| anyhow::anyhow!("未知物编表类型: {}", t_str))?;
                let table = EditorTable::open(&ctx, map_name, tt)?;
                let summaries = table.list_summaries()?;
                Ok(json!({
                    "project_id": ws.project_id,
                    "table_type": tt.name_cn(),
                    "count": summaries.len(),
                    "items": summaries
                }))
            }
            "editor_table_get" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let t_str = args
                    .get("tableType")
                    .or_else(|| args.get("table_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unit");
                let key = args
                    .get("key")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("缺少物编 key 参数"))?;
                let tt = TableType::parse(t_str)
                    .ok_or_else(|| anyhow::anyhow!("未知物编表类型: {}", t_str))?;
                let table = EditorTable::open(&ctx, map_name, tt)?;
                let obj = table.get(key)?;
                Ok(json!({
                    "project_id": ws.project_id,
                    "table_type": tt.name_cn(),
                    "key": obj.key,
                    "name": obj.name,
                    "data": obj.json_data
                }))
            }
            "editor_table_upsert" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let t_str = args
                    .get("tableType")
                    .or_else(|| args.get("table_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unit");
                let tt = TableType::parse(t_str)
                    .ok_or_else(|| anyhow::anyhow!("未知物编表类型: {}", t_str))?;
                let table = EditorTable::open(&ctx, map_name, tt)?;

                let key = args.get("key").and_then(|v| v.as_u64());
                let name = args.get("name").and_then(|v| v.as_str());
                let copy_from = args
                    .get("copyFromKey")
                    .or_else(|| args.get("copy_from_key"))
                    .and_then(|v| v.as_u64());
                let fields = args.get("fields").and_then(|v| v.as_object());
                let kv = args.get("kv").and_then(|v| v.as_object());

                let obj = if let Some(k) = key {
                    if table.list_keys()?.contains(&k) {
                        let mut f_merged = fields.cloned().unwrap_or_else(Map::new);
                        if let Some(n) = name {
                            f_merged.insert("name".to_string(), Value::String(n.to_string()));
                        }
                        table.update(k, Some(&f_merged), kv)?
                    } else {
                        table.create(
                            Some(k),
                            name.unwrap_or(&k.to_string()),
                            copy_from,
                            fields,
                            kv,
                        )?
                    }
                } else {
                    table.create(
                        None,
                        name.unwrap_or("新建物编"),
                        copy_from,
                        fields,
                        kv,
                    )?
                };

                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "table_type": tt.name_cn(),
                    "key": obj.key,
                    "name": obj.name,
                    "file_path": obj.file_path
                }))
            }
            "editor_table_delete" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let t_str = args
                    .get("tableType")
                    .or_else(|| args.get("table_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unit");
                let key = args
                    .get("key")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("缺少物编 key 参数"))?;
                let tt = TableType::parse(t_str)
                    .ok_or_else(|| anyhow::anyhow!("未知物编表类型: {}", t_str))?;
                let table = EditorTable::open(&ctx, map_name, tt)?;
                let deleted = table.delete(key)?;
                Ok(json!({
                    "success": deleted,
                    "project_id": ws.project_id,
                    "table_type": tt.name_cn(),
                    "key": key
                }))
            }
            "excel_import" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let rule: ExcelImportRule = serde_json::from_value(args.clone())?;
                let res = apply_excel_rule(&ctx, map_name, &rule)?;
                Ok(serde_json::to_value(res)?)
            }
            "enable_global_script" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let files = GlobalScriptManager::enable(&ctx)?;
                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "updated_files": files
                }))
            }
            "sync_cloud_script" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let enable = args.get("enable").and_then(|v| v.as_bool()).unwrap_or(true);
                let wait = args
                    .get("waitDebugger")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let path = CloudScriptManager::sync_cloud_script(&ctx, enable, None, None, wait)?;
                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "cloud_script_path": path
                }))
            }
            "reload_editor_table" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let clients = ws.console_server.list_clients();
                for c in &clients {
                    c.notify("command", json!({ "data": ".re" }));
                }
                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "reloaded_clients": clients.len(),
                    "message": format!("已向项目 {} 的 {} 个客户端发送物编热更指令", ws.project_id, clients.len())
                }))
            }
            "reload_trigger" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let rep = EcaCompiler::compile_map(&ctx, map_name, None)?;
                let count = ws.console_server.reload_lua_all();
                Ok(json!({
                    "success": true,
                    "project_id": ws.project_id,
                    "compiled": rep,
                    "reloaded_clients": count
                }))
            }
            "custom_define_overview" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let unit_attrs = CustomDefine::get_unit_attrs(&ctx, map_name).unwrap_or_default();
                let player_attrs = CustomDefine::get_player_attrs(&ctx, map_name).unwrap_or_default();
                let custom_events = CustomDefine::get_custom_events(&ctx, map_name).unwrap_or_default();
                let jump_words = CustomDefine::get_jumpwords(&ctx, map_name).unwrap_or_default();
                let fonts = CustomDefine::get_fonts(&ctx, map_name).unwrap_or_default();
                let ui_anims = CustomDefine::get_ui_anims(&ctx, map_name).unwrap_or_default();
                Ok(json!({
                    "project_id": ws.project_id,
                    "map_name": map_name.unwrap_or(&ctx.current_map_name),
                    "unit_attrs": unit_attrs,
                    "player_attrs": player_attrs,
                    "custom_events": custom_events,
                    "jump_words": jump_words,
                    "fonts": fonts,
                    "ui_anims": ui_anims,
                }))
            }
            "plugin_manage" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
                let func = args.get("func").and_then(|v| v.as_str()).unwrap_or("onSave");
                match action {
                    "init" => {
                        let dir = PluginManager::init_plugins(&ctx, map_name)?;
                        Ok(json!({ "success": true, "plugin_dir": dir }))
                    }
                    "list" => {
                        let list = PluginManager::list_plugins(&ctx, map_name)?;
                        Ok(json!({ "success": true, "plugins": list }))
                    }
                    "run" => {
                        let out = PluginManager::run_plugin_function(&ctx, map_name, None, func)?;
                        Ok(json!({ "success": true, "outputs": out }))
                    }
                    other => anyhow::bail!("不支持的插件操作: {}", other),
                }
            }
            "generate_lua_doc" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let ctx = ws.context.read().clone();
                let ed = self.manager.editor_env();
                let doc_json = ed.as_ref().and_then(|e| {
                    e.editor_exe_path
                        .parent()
                        .map(|p| p.join("script").join("doc.json"))
                        .filter(|p| p.is_file())
                });
                let files = LuaDocGenerator::generate_from_doc_json(&ctx, map_name, doc_json.as_deref())?;
                Ok(json!({ "success": true, "generated_files": files }))
            }
            "start_network_server" => {
                let (p1, p2) = NetworkTestServer::start_both().await?;
                Ok(json!({
                    "success": true,
                    "echo_port": p1,
                    "tick_port": p2,
                    "message": format!("本地网络测试服务器已启动 (127.0.0.1:{} & 127.0.0.1:{})", p1, p2)
                }))
            }
            "open_in_explorer" => {
                let ws = self.manager.resolve_project(proj_sel).await?;
                let _ = Command::new("explorer.exe").arg(&ws.project_path).spawn();
                Ok(json!({ "success": true, "opened": ws.project_path }))
            }
            "pick_folder_dialog" => {
                let picked = tokio::task::spawn_blocking(|| {
                    rfd::FileDialog::new()
                        .set_title("请选择 Y3 地图工程根目录或地图文件夹 (含 header.project 或 header.map)")
                        .pick_folder()
                })
                .await?;
                if let Some(p) = picked {
                    Ok(json!({ "path": p.to_string_lossy() }))
                } else {
                    Ok(json!({ "cancelled": true }))
                }
            }
            "pick_file_dialog" => {
                let filter_raw = args
                    .get("filter")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let picked = tokio::task::spawn_blocking(move || {
                    let mut dlg = rfd::FileDialog::new();
                    if filter_raw.contains("Editor.exe") {
                        dlg = dlg
                            .set_title("请选择 Y3 编辑器的 Editor.exe 或 Game_x64h.exe")
                            .add_filter("Y3 Editor", &["exe"]);
                    } else if filter_raw.contains("xlsx") {
                        dlg = dlg
                            .set_title("请选择 Excel 导表文件 (.xlsx)")
                            .add_filter("Excel Files", &["xlsx", "xls"]);
                    }
                    dlg.pick_file()
                })
                .await?;
                if let Some(p) = picked {
                    Ok(json!({ "path": p.to_string_lossy() }))
                } else {
                    Ok(json!({ "cancelled": true }))
                }
            }
            "discover_editor_mcps" => {
                let list = discover_running_editor_mcps();
                Ok(json!({
                    "count": list.len(),
                    "editors": list
                }))
            }
            "call_y3editor_tool" => {
                let editor_tool = args
                    .get("toolName")
                    .or_else(|| args.get("tool_name"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("缺少 toolName 参数（例如 get_map_info、save_editor、hotfix_object_editor）"))?;
                let editor_args = args
                    .get("toolArgs")
                    .or_else(|| args.get("tool_args"))
                    .or_else(|| args.get("arguments"))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let explicit_port = args
                    .get("editorPort")
                    .or_else(|| args.get("editor_port"))
                    .and_then(|v| v.as_u64())
                    .map(|p| p as u16);

                let target_port = if let Some(p) = explicit_port {
                    p
                } else {
                    let target_proj_path = if let Some(sel) = proj_sel {
                        self.manager
                            .resolve_project(Some(sel))
                            .await
                            .ok()
                            .map(|ws| ws.project_path.clone())
                    } else {
                        self.manager
                            .resolve_project(None)
                            .await
                            .ok()
                            .map(|ws| ws.project_path.clone())
                    };
                    resolve_editor_mcp_port(target_proj_path.as_deref(), map_name)?
                };

                let rpc_req = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": editor_tool,
                        "arguments": editor_args
                    }
                });
                let resp = forward_jsonrpc_to_editor_mcp(target_port, &rpc_req)?;
                Ok(json!({
                    "success": true,
                    "editor_port": target_port,
                    "tool_name": editor_tool,
                    "response": resp
                }))
            }
            "discover_runtime_mcps" => {
                let list = self.discover_running_runtime_mcps();
                Ok(json!({
                    "count": list.len(),
                    "runtimes": list
                }))
            }
            "trigger_ui_touch_event_by_path" | "call_y3runtime_tool" => {
                let tool_name = args
                    .get("toolName")
                    .or_else(|| args.get("tool_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("trigger_ui_touch_event_by_path");
                let ui_path = args
                    .get("ui_path")
                    .or_else(|| args.get("uiPath"))
                    .or_else(|| args.get("toolArgs").and_then(|a| a.get("ui_path")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let touch = args
                    .get("touch")
                    .or_else(|| args.get("toolArgs").and_then(|a| a.get("touch")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("左键");
                let slot = args
                    .get("clientSlot")
                    .or_else(|| args.get("client_slot"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                let explicit_port = args
                    .get("runtimePort")
                    .or_else(|| args.get("runtime_port"))
                    .and_then(|v| v.as_u64())
                    .map(|p| p as u16);

                self.execute_y3runtime_action(proj_sel, slot, explicit_port, tool_name, ui_path, touch)
                    .await
            }
            _ => anyhow::bail!("未知工具名: {}", name),
        }
    }

    /// 返回完整的 MCP 工具 Schema 列表（所有工具均支持可选 `projectPath` 与 `mapName` 以支持多项目多地图并发操作）
    pub fn list_mcp_tools() -> Value {
        let project_props = json!({
            "projectPath": {
                "type": "string",
                "description": "Optional path or ID of the target Y3 project (enables multi-project concurrent control)"
            },
            "mapName": {
                "type": "string",
                "description": "Optional sub-map name under the target project (e.g., 'EntryMap')"
            }
        });

        let make_schema = |extra: Value, required: Vec<&str>| -> Value {
            let mut props = project_props.as_object().unwrap().clone();
            if let Some(obj) = extra.as_object() {
                for (k, v) in obj {
                    props.insert(k.clone(), v.clone());
                }
            }
            json!({
                "type": "object",
                "properties": props,
                "required": required,
            })
        };

        json!([
            {
                "name": "set_project_path",
                "description": "Bind or register a Y3 map project directory (supports registering multiple projects simultaneously).",
                "inputSchema": make_schema(json!({
                    "setActive": { "type": "boolean" }
                }), vec!["projectPath"])
            },
            {
                "name": "list_projects",
                "description": "List all registered Y3 projects, their sub-maps, dedicated ConsoleServer ports, and game session states.",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "remove_project",
                "description": "Remove a registered project and stop its dedicated ConsoleServer.",
                "inputSchema": make_schema(json!({}), vec!["projectPath"])
            },
            {
                "name": "list_maps",
                "description": "List all sub-maps under a Y3 project.",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "switch_map",
                "description": "Switch the active sub-map for a Y3 project.",
                "inputSchema": make_schema(json!({}), vec!["mapName"])
            },
            {
                "name": "get_helper_status",
                "description": "Get current y3-helper environment status including all bound Y3 projects, maps, and engine executable path (Game_x64h.exe). Note: Always use launch_game / launch_editor tools or Game_x64h.exe instead of passing CLI args directly to Editor.exe.",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "init_project",
                "description": "Initialize a new Y3 map project (clones y3-lualib into script/y3, copies template config, and clones .y3maker).",
                "inputSchema": make_schema(json!({
                    "repoSource": { "type": "string", "enum": ["gitee", "github", "english"] }
                }), vec![])
            },
            {
                "name": "launch_game",
                "description": "Launch the game for a specific project and sub-map (supports single-player and multi-player modes).",
                "inputSchema": make_schema(json!({
                    "multiMode": { "type": "boolean" },
                    "multiPlayers": { "type": "array", "items": { "type": "integer" } },
                    "tracy": { "type": "boolean" }
                }), vec![])
            },
            {
                "name": "get_game_status",
                "description": "Get the current game status and connected client window slots for a project.",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "execute_lua",
                "description": "Execute Lua code in the running game of a project. Use clientSlot to select a specific game window.",
                "inputSchema": make_schema(json!({
                    "code": { "type": "string" },
                    "clientSlot": { "type": "integer" }
                }), vec!["code"])
            },
            {
                "name": "reload_lua",
                "description": "Send Lua hot-reload (.rd) command to all connected Y3 game clients of a project.",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "quick_restart",
                "description": "Quickly restart (.rr) the game session for a project or specific clientSlot.",
                "inputSchema": make_schema(json!({
                    "clientSlot": { "type": "integer" }
                }), vec![])
            },
            {
                "name": "stop_game",
                "description": "Stop the running game session for a project or specific clientSlot.",
                "inputSchema": make_schema(json!({
                    "clientSlot": { "type": "integer" }
                }), vec![])
            },
            {
                "name": "kill_game",
                "description": "Force terminate running Y3 game processes (Game_x64h.exe) for the target project (or all projects if killAll=true).",
                "inputSchema": make_schema(json!({
                    "killAll": { "type": "boolean" }
                }), vec![])
            },
            {
                "name": "launch_editor",
                "description": "Open the target Y3 map project in the Y3 Editor (via Game_x64h.exe with Base64-encoded project_path; never pass CLI args directly to Editor.exe).",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "get_logs",
                "description": "Fetch recent game logs from a project's game window.",
                "inputSchema": make_schema(json!({
                    "limit": { "type": "integer" },
                    "clientSlot": { "type": "integer" }
                }), vec![])
            },
            {
                "name": "capture_screenshot",
                "description": "Capture a screenshot from the running game of a project.",
                "inputSchema": make_schema(json!({
                    "clientSlot": { "type": "integer" }
                }), vec![])
            },
            {
                "name": "read_problems_lua",
                "description": "Run Lua diagnostics on project files.",
                "inputSchema": make_schema(json!({
                    "pathGlob": {}
                }), vec![])
            },
            {
                "name": "get_ui_canvas",
                "description": "Return the map UI canvas tree as formatted text, including node names, control types, and UIDs.",
                "inputSchema": make_schema(json!({
                    "nodePath": { "type": "string" },
                    "depth": { "type": "integer" }
                }), vec![])
            },
            {
                "name": "compile_eca",
                "description": "Compile ECA triggers, functions, global variables, and object triggers into Lua code for a Y3 map.",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "generate_meta",
                "description": "Generate Lua & TypeScript meta definition files (unitAttrs, playerAttrs, customEvents, ui, uiAnim, jumpword, font, objects).",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "save_gmp",
                "description": "Rebuild and save the binary .gmp map file from editor_table/, ui/, and language JSON files.",
                "inputSchema": make_schema(json!({
                    "updatePrefabs": { "type": "boolean" },
                    "updateUI": { "type": "boolean" },
                    "updateUSLanguage": { "type": "boolean" },
                    "updateZHLanguage": { "type": "boolean" }
                }), vec![])
            },
            {
                "name": "editor_table_list",
                "description": "List all object summaries in an editor table (unit, ability, modifier, item, projectile, destructible, decoration, sound, technology).",
                "inputSchema": make_schema(json!({
                    "tableType": { "type": "string" }
                }), vec!["tableType"])
            },
            {
                "name": "editor_table_get",
                "description": "Read a single object's full JSON and localized fields from an editor table.",
                "inputSchema": make_schema(json!({
                    "tableType": { "type": "string" },
                    "key": { "type": "integer" }
                }), vec!["tableType", "key"])
            },
            {
                "name": "editor_table_upsert",
                "description": "Create or update an object in an editor table (with automatic MurmurHash3 localization and Y3Json formatting).",
                "inputSchema": make_schema(json!({
                    "tableType": { "type": "string" },
                    "key": { "type": "integer" },
                    "name": { "type": "string" },
                    "copyFromKey": { "type": "integer" },
                    "fields": { "type": "object" },
                    "kv": { "type": "object" }
                }), vec!["tableType"])
            },
            {
                "name": "editor_table_delete",
                "description": "Delete an object by key from an editor table.",
                "inputSchema": make_schema(json!({
                    "tableType": { "type": "string" },
                    "key": { "type": "integer" }
                }), vec!["tableType", "key"])
            },
            {
                "name": "discover_editor_mcps",
                "description": "Discover all running Y3 Editor instances and their built-in y3editor (game_editor_mcp) MCP ports, bound project paths, and map names.",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "call_y3editor_tool",
                "description": "Route and call a tool on the running Y3 Editor's built-in y3editor MCP (e.g., get_map_info, save_editor, hotfix_object_editor, hotfix_ui_editor, terrain_*, entity_*), automatically matching the editor instance for the target projectPath.",
                "inputSchema": make_schema(json!({
                    "toolName": { "type": "string", "description": "Name of the y3editor MCP tool to call" },
                    "toolArgs": { "type": "object", "description": "Arguments for the y3editor MCP tool" },
                    "editorPort": { "type": "integer", "description": "Optional explicit y3editor MCP port" }
                }), vec!["toolName"])
            },
            {
                "name": "discover_runtime_mcps",
                "description": "Discover all running Y3 game runtime instances, their PIDs, ConsoleServer client slots, and built-in y3runtime MCP ports.",
                "inputSchema": make_schema(json!({}), vec![])
            },
            {
                "name": "trigger_ui_touch_event_by_path",
                "description": "Trigger a UI touch/click event by widget path in a running Y3 game window (compatible with y3runtime MCP, plus multi-project projectPath and multi-window clientSlot routing).",
                "inputSchema": make_schema(json!({
                    "ui_path": { "type": "string", "description": "Full UI widget path, e.g. 'panel_1.button_1'" },
                    "touch": { "type": "string", "description": "Touch button type, e.g. '左键' or '右键'" },
                    "clientSlot": { "type": "integer", "description": "Optional target game window slot in multi-player mode" },
                    "runtimePort": { "type": "integer", "description": "Optional explicit y3runtime MCP port" }
                }), vec!["ui_path"])
            }
        ])
    }

    /// 处理单条 JSON-RPC 2.0 消息（供 Streamable HTTP 与 Stdio MCP 共用）
    pub async fn handle_jsonrpc_message(&self, msg: &Value) -> Option<Value> {
        let method = msg.get("method").and_then(|v| v.as_str())?;
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

        // Notification（无 id）无需返回响应
        if id.is_none() {
            return None;
        }
        let req_id = id.unwrap();

        match method {
            "initialize" => Some(json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "y3-helper",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": MCP_INSTRUCTIONS
                }
            })),
            "ping" => Some(json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {}
            })),
            "tools/list" => Some(json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "tools": Self::list_mcp_tools()
                }
            })),
            "tools/call" => {
                let tool_name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tool_args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));

                match self.call_tool(tool_name, &tool_args).await {
                    Ok(res) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "content": [{
                                "type": "text",
                                "text": serde_json::to_string_pretty(&res).unwrap_or_default()
                            }]
                        }
                    })),
                    Err(e) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "content": [{
                                "type": "text",
                                "text": format!("Error: {}", e)
                            }],
                            "isError": true
                        }
                    })),
                }
            }
            _ => Some(json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {}", method)
                }
            })),
        }
    }

    /// 扫描本机所有运行中的游戏实例及其关联的 `y3runtime` MCP 端口与 ConsoleServer 窗口槽位
    pub fn discover_running_runtime_mcps(&self) -> Vec<Value> {
        let mut results = Vec::new();
        let listening_map = list_game_exe_listening_ports();
        let editor_ports: HashSet<u16> = discover_running_editor_mcps()
            .iter()
            .filter_map(|e| e.get("port").and_then(|v| v.as_u64()).map(|n| n as u16))
            .collect();

        let workspaces = self.manager.list_workspaces();
        let mut matched_ports = HashSet::new();

        for ws in &workspaces {
            let pids = ws.session_manager.known_pids();
            let status = ws.session_manager.get_game_status();
            let clients = status
                .get("clients")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut runtime_ports = Vec::new();
            for pid in &pids {
                if let Some(ports) = listening_map.get(pid) {
                    for &p in ports {
                        if is_candidate_mcp_http_port(p) && !editor_ports.contains(&p) && probe_runtime_mcp_port(p) {
                            runtime_ports.push(json!({ "pid": pid, "port": p, "mcp_url": format!("http://127.0.0.1:{}/mcp", p) }));
                            matched_ports.insert(p);
                        }
                    }
                }
            }

            if !pids.is_empty() || !clients.is_empty() {
                results.push(json!({
                    "project_id": ws.project_id,
                    "project_path": ws.project_path,
                    "console_port": ws.console_server.port,
                    "known_pids": pids,
                    "connected_clients": clients,
                    "runtime_mcp_ports": runtime_ports,
                    "lua_fallback_ready": !clients.is_empty()
                }));
            }
        }

        // 检查默认 8767 或其他未归属到具体项目的 Game_x64h.exe runtime 端口
        let mut extra_candidates: Vec<(u16, Option<u32>)> = vec![(8767, None)];
        for (pid, ports) in &listening_map {
            for &p in ports {
                if is_candidate_mcp_http_port(p) && !editor_ports.contains(&p) && !matched_ports.contains(&p) {
                    extra_candidates.push((p, Some(*pid)));
                }
            }
        }
        for (port, pid_opt) in extra_candidates {
            if is_candidate_mcp_http_port(port) && !matched_ports.contains(&port) && !editor_ports.contains(&port) && probe_runtime_mcp_port(port) {
                matched_ports.insert(port);
                results.push(json!({
                    "project_id": "standalone_runtime",
                    "project_path": null,
                    "console_port": null,
                    "known_pids": pid_opt.into_iter().collect::<Vec<u32>>(),
                    "connected_clients": [],
                    "runtime_mcp_ports": [{
                        "pid": pid_opt,
                        "port": port,
                        "mcp_url": format!("http://127.0.0.1:{}/mcp", port)
                    }],
                    "lua_fallback_ready": false
                }));
            }
        }

        results
    }

    /// 执行 `y3runtime` 工具（优先路由到目标项目的原生 `y3runtime` 端口；若多项目/多开窗口未绑定端口则自动通过专属 Lua 通道精准触发）
    pub async fn execute_y3runtime_action(
        &self,
        proj_sel: Option<&str>,
        client_slot: Option<usize>,
        explicit_port: Option<u16>,
        tool_name: &str,
        ui_path: &str,
        touch: &str,
    ) -> anyhow::Result<Value> {
        let ws_opt = self.manager.resolve_project(proj_sel).await.ok();

        // 1. 确定候选原生 y3runtime MCP 端口（若未指定非0多开槽位）
        let mut target_runtime_port = explicit_port;
        if target_runtime_port.is_none() && client_slot.unwrap_or(0) == 0 {
            let listening_map = list_game_exe_listening_ports();
            if let Some(ws) = &ws_opt {
                for pid in ws.session_manager.known_pids() {
                    if let Some(ports) = listening_map.get(&pid) {
                        for &p in ports {
                            if is_candidate_mcp_http_port(p) && probe_runtime_mcp_port(p) {
                                target_runtime_port = Some(p);
                                break;
                            }
                        }
                    }
                    if target_runtime_port.is_some() {
                        break;
                    }
                }
            }
            if target_runtime_port.is_none() && probe_runtime_mcp_port(8767) {
                target_runtime_port = Some(8767);
            }
        }

        // 2. 引擎 A：若匹配到了原生 y3runtime MCP 端口，直接带 Session 握手转发调用
        if let Some(port) = target_runtime_port {
            let args_json = json!({
                "ui_path": ui_path,
                "touch": touch
            });
            if let Ok(resp) = call_mcp_http_with_session(
                port,
                "tools/call",
                json!({
                    "name": tool_name,
                    "arguments": args_json
                }),
            ) {
                return Ok(json!({
                    "success": true,
                    "engine": "native_y3runtime_mcp",
                    "runtime_port": port,
                    "ui_path": ui_path,
                    "touch": touch,
                    "response": resp
                }));
            }
        }

        // 3. 引擎 B（多项目并发 / 多人多开窗口槽位 Lua 通道直达）：
        let ws = ws_opt.ok_or_else(|| {
            anyhow::anyhow!(
                "当前未检测到运行中的 y3runtime (8767)，且未选中已启动游戏的 Y3 项目。请先启动游戏！"
            )
        })?;

        let lua_script = build_lua_ui_touch_script(ui_path, touch);
        let exec_res = ws.session_manager.execute_lua(&lua_script, client_slot).await;
        let ok = exec_res.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok {
            anyhow::bail!(
                "原生 y3runtime 端口未就绪，且通过项目 [{}] Lua 通道触发 UI 事件失败: {}",
                ws.project_id,
                exec_res
            );
        }

        Ok(json!({
            "success": true,
            "engine": "multi_project_lua_channel",
            "project_id": ws.project_id,
            "client_slot": client_slot.unwrap_or(0),
            "ui_path": ui_path,
            "touch": touch,
            "lua_result": exec_res
        }))
    }
}

#[derive(Clone)]
struct HttpAppState {
    dispatcher: Arc<ToolDispatcher>,
    sessions: Arc<RwLock<HashSet<String>>>,
    port: u16,
}

/// 启动 MCP Streamable HTTP + REST API 服务（默认监听 `127.0.0.1:8766`）
pub async fn start_http_server(
    dispatcher: Arc<ToolDispatcher>,
    port: u16,
) -> anyhow::Result<u16> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let actual_port = listener.local_addr()?.port();

    let state = HttpAppState {
        dispatcher,
        sessions: Arc::new(RwLock::new(HashSet::new())),
        port: actual_port,
    };

    let app = Router::new()
        .route("/", get(handle_desktop_ui))
        .route("/health", get(handle_health))
        .route("/mcp", post(handle_mcp_post).delete(handle_mcp_delete))
        .route("/y3editor/mcp", post(handle_y3editor_mcp_proxy))
        .route("/y3runtime/mcp", post(handle_y3runtime_mcp_proxy).delete(handle_mcp_delete))
        .route("/api/projects", get(handle_api_projects))
        .route("/api/status", get(handle_api_projects))
        .route("/api/editor-mcps", get(handle_api_editor_mcps))
        .route("/api/runtime-mcps", get(handle_api_runtime_mcps))
        .route("/api/call", post(handle_api_call))
        .with_state(state);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(actual_port)
}

async fn handle_desktop_ui() -> Html<&'static str> {
    Html(DESKTOP_UI_HTML)
}

/// 启动独立桌面客户端应用窗口（使用独立 user-data-dir 的 Edge App 模式，呈现无地址栏原生窗口体验且不与现有浏览器冲突）
pub fn launch_desktop_gui_window(port: u16) {
    let url = format!("http://127.0.0.1:{}", port);
    let profile_dir = std::env::temp_dir().join("y3-helper-desktop-profile");
    let _ = std::fs::create_dir_all(&profile_dir);
    let user_data_arg = format!("--user-data-dir={}", profile_dir.display());
    let app_arg = format!("--app={}", url);

    let edge_candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    for edge in edge_candidates {
        if Path::new(edge).is_file() {
            if Command::new(edge)
                .args([
                    &user_data_arg,
                    &app_arg,
                    "--window-size=1480,920",
                    "--no-first-run",
                    "--no-default-browser-check",
                ])
                .spawn()
                .is_ok()
            {
                return;
            }
        }
    }
    let _ = Command::new("cmd").args(["/C", "start", "", &url]).spawn();
}

async fn handle_health(State(state): State<HttpAppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "transport": "streamable-http",
        "port": state.port,
        "activeSessions": state.sessions.read().len(),
        "projects": state.dispatcher.manager.list_projects_json()
    }))
}

async fn handle_api_projects(State(state): State<HttpAppState>) -> Json<Value> {
    Json(state.dispatcher.manager.list_projects_json())
}

async fn handle_api_call(
    State(state): State<HttpAppState>,
    Json(payload): Json<Value>,
) -> Response {
    let tool = payload
        .get("tool")
        .or_else(|| payload.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let args = payload
        .get("args")
        .or_else(|| payload.get("arguments"))
        .cloned()
        .unwrap_or_else(|| payload.clone());

    match state.dispatcher.call_tool(tool, &args).await {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "success": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn handle_mcp_post(
    State(state): State<HttpAppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    let mut session_id = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if payload.get("method").and_then(|v| v.as_str()) == Some("initialize") && session_id.is_none()
    {
        let new_id = uuid::Uuid::new_v4().to_string();
        state.sessions.write().insert(new_id.clone());
        session_id = Some(new_id);
    }

    match state.dispatcher.handle_jsonrpc_message(&payload).await {
        Some(reply) => {
            let mut resp = (StatusCode::OK, Json(reply)).into_response();
            if let Some(sid) = session_id {
                if let Ok(hv) = sid.parse() {
                    resp.headers_mut().insert("Mcp-Session-Id", hv);
                }
            }
            resp
        }
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn handle_mcp_delete(
    State(state): State<HttpAppState>,
    headers: HeaderMap,
) -> StatusCode {
    if let Some(sid) = headers.get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        state.sessions.write().remove(sid);
    }
    StatusCode::NO_CONTENT
}

/// 运行 Stdio 模式的 MCP Server（从 stdin 逐行读取 JSON-RPC 2.0 消息并写入 stdout）
pub async fn run_stdio_mcp_server(dispatcher: Arc<ToolDispatcher>) -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in BufReader::new(stdin.lock()).lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": "Parse error" }
                });
                writeln!(stdout, "{}", err_resp)?;
                stdout.flush()?;
                continue;
            }
        };
        if let Some(reply) = dispatcher.handle_jsonrpc_message(&msg).await {
            writeln!(stdout, "{}", serde_json::to_string(&reply)?)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

async fn handle_api_editor_mcps() -> Json<Value> {
    let list = discover_running_editor_mcps();
    Json(json!({
        "count": list.len(),
        "editors": list
    }))
}

/// 智能多编辑器 `y3editor` MCP 路由网关 (`POST /y3editor/mcp`)
/// 支持通过请求 Header `X-Y3-Project`、或 `tools/call` 参数中的 `projectPath`、或当前客户端选中的活动项目，
/// 自动将 `y3editor` MCP 请求路由到对应项目的 Y3 编辑器窗口 MCP 端口。
async fn handle_y3editor_mcp_proxy(
    State(state): State<HttpAppState>,
    headers: HeaderMap,
    Json(mut payload): Json<Value>,
) -> Response {
    let header_proj = headers
        .get("x-y3-project")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let header_port = headers
        .get("x-y3-editor-port")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u16>().ok());

    // 从 tools/call arguments 中提取并剥离可选的 projectPath / editorPort（避免传给原生 y3editor 报多余参数错误）
    let mut arg_proj: Option<String> = None;
    let mut arg_map: Option<String> = None;
    let mut arg_port: Option<u16> = None;
    if let Some(args_obj) = payload
        .get_mut("params")
        .and_then(|p| p.get_mut("arguments"))
        .and_then(|a| a.as_object_mut())
    {
        if let Some(v) = args_obj.remove("editorPort").or_else(|| args_obj.remove("editor_port")) {
            arg_port = v.as_u64().map(|n| n as u16);
        }
        if let Some(v) = args_obj.remove("projectPath").or_else(|| args_obj.remove("project_id")) {
            arg_proj = v.as_str().map(|s| s.to_string());
        }
        if let Some(v) = args_obj.get("mapName").or_else(|| args_obj.get("map_name")) {
            arg_map = v.as_str().map(|s| s.to_string());
        }
    }

    let target_port = if let Some(p) = header_port.or(arg_port) {
        p
    } else {
        let proj_sel = header_proj.as_deref().or(arg_proj.as_deref());
        let resolved_path = state
            .dispatcher
            .manager
            .resolve_project(proj_sel)
            .await
            .ok()
            .map(|ws| ws.project_path.clone());
        match resolve_editor_mcp_port(resolved_path.as_deref(), arg_map.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                let req_id = payload.get("id").cloned().unwrap_or(Value::Null);
                return (
                    StatusCode::OK,
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {
                            "code": -32000,
                            "message": e.to_string()
                        }
                    })),
                )
                    .into_response();
            }
        }
    };

    match forward_jsonrpc_to_editor_mcp(target_port, &payload) {
        Ok(resp_val) => (StatusCode::OK, Json(resp_val)).into_response(),
        Err(e) => {
            let req_id = payload.get("id").cloned().unwrap_or(Value::Null);
            (
                StatusCode::OK,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {
                        "code": -32001,
                        "message": format!("转发到编辑器 MCP (127.0.0.1:{}) 失败: {}", target_port, e)
                    }
                })),
            )
                .into_response()
        }
    }
}

/// 判断端口是否为候选的 HTTP MCP 端口（排除 `>= 49152` 的 `temp_ipc_server` 动态 pickle 端口、8768 网关自身以及 8769 `bridge_server` 二进制端口）
fn is_candidate_mcp_http_port(port: u16) -> bool {
    (8000..49152).contains(&port) && port != 8768 && port != 8769
}

/// 当检测到多个 Y3 编辑器 (`Game_x64h.exe`) 因官方 `dm/clients/mcp/mcp_server.py` 开启 `SO_REUSEADDR`
/// 而重复绑定在同一个 `127.0.0.1:8765` 端口（导致后启动编辑器的 8765 套接字被 Windows 内核屏蔽）时，
/// 通过被屏蔽编辑器的 `temp_ipc_server` 端口自动将其内置 `mcp_server` 迁移至独立空闲端口（8775..=8799）。
fn auto_rebind_shadowed_editor_mcps(listening_map: &std::collections::HashMap<u32, Vec<u16>>) -> bool {
    let mut pids_only_on_8765: Vec<(u32, u16)> = Vec::new();
    let mut used_ports: HashSet<u16> = HashSet::new();

    for (&pid, ports) in listening_map {
        for &p in ports {
            used_ports.insert(p);
        }
        let has_8765 = ports.contains(&8765);
        let has_dedicated_mcp = ports
            .iter()
            .any(|&p| is_candidate_mcp_http_port(p) && p != 8765 && p != 8767);
        let ipc_port = ports.iter().copied().find(|&p| p >= 49152);
        if has_8765 && !has_dedicated_mcp {
            if let Some(ipc) = ipc_port {
                pids_only_on_8765.push((pid, ipc));
            }
        }
    }

    // 只有当存在 2 个及以上编辑器同时挤在 8765 端口上时才需要自动分流重绑
    if pids_only_on_8765.len() < 2 {
        return false;
    }

    // 探测当前在 8765 上实际应答的编辑器项目路径（保持该持有者不动，只迁移被屏蔽的编辑器）
    let active_8765_proj = probe_editor_mcp_port(8765, None)
        .and_then(|v| v.get("project_path").and_then(|s| s.as_str()).map(normalize_path_key))
        .unwrap_or_default();
    if active_8765_proj.is_empty() {
        return false;
    }

    let mut rebound_any = false;
    let mut next_port = 8775u16;

    for (_pid, ipc_port) in pids_only_on_8765 {
        while used_ports.contains(&next_port)
            || std::net::TcpListener::bind(("127.0.0.1", next_port)).is_err()
        {
            next_port += 1;
            if next_port > 8799 {
                return rebound_any;
            }
        }
        let assigned_port = next_port;
        used_ports.insert(assigned_port);

        if trigger_editor_mcp_rebind_via_ipc(ipc_port, &active_8765_proj, assigned_port) {
            rebound_any = true;
        }
    }

    if rebound_any {
        // 等待后台线程完成新端口 HTTP Server 绑定
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(80));
            if probe_editor_mcp_port(8775, None).is_some() {
                break;
            }
        }
    }
    rebound_any
}

/// 向编辑器进程 `temp_ipc_server` 端口发送零残留的维护指令：
/// 1. 清理 `MultiServer.connections` 中可能残留的断开连接；
/// 2. 若当前编辑器不是 8765 的实际应答者（即处于被 `SO_REUSEADDR` 屏蔽状态），将其 `clients.mcp.mcp_server` 迁移至 `new_port`。
fn trigger_editor_mcp_rebind_via_ipc(ipc_port: u16, active_8765_proj_norm: &str, new_port: u16) -> bool {
    use std::io::Read;
    use std::net::{Shutdown, SocketAddr, TcpStream};
    use std::time::Duration;

    let addr: SocketAddr = match format!("127.0.0.1:{}", ipc_port).parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(120)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));

    let py_expr = format!(
        "([obj.connections.clear() for obj in __import__('gc').get_objects() if type(obj).__name__ == 'MultiServer'], \
          (lambda ms, info: __import__('threading').Thread(target=lambda: (ms.stop_server(), ms.start_server('127.0.0.1', {port})), daemon=True).start() \
           if ms and getattr(ms._server, 'server_port', 0) == 8765 and info.get('project_path', '').replace('\\\\', '/').rstrip('/').lower() != '{active}' else None)\
          (__import__('sys').modules.get('clients.mcp.mcp_server'), \
           __import__('json').loads(__import__('sys').modules['clients.mcp.mcp_server']._call_tool('get_map_info', {{}}))), \
          {{'proto': '', 'data': None}})[2]",
        port = new_port,
        active = active_8765_proj_norm.replace('\'', "\\'")
    );
    let pickle_payload = format!("cbuiltins\neval\n(S\"{}\"\ntR.", py_expr);
    let raw_bytes = pickle_payload.as_bytes();
    let total_len = (raw_bytes.len() + 4) as u32;

    let mut packet = Vec::with_capacity(raw_bytes.len() + 4);
    packet.extend_from_slice(&total_len.to_le_bytes());
    packet.extend_from_slice(raw_bytes);

    if stream.write_all(&packet).is_err() || stream.flush().is_err() {
        return false;
    }

    // 读取 accept 时编辑器发送的初始化字节并等待一帧（~33ms）让协程执行完毕，避免触发 TCP RST (WinError 10053)
    std::thread::sleep(Duration::from_millis(80));
    let mut drain_buf = [0u8; 4096];
    while let Ok(n) = stream.read(&mut drain_buf) {
        if n == 0 {
            break;
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    true
}

/// 扫描本机所有正在运行的 Y3 编辑器实例及其内置 `y3editor` (`game_editor_mcp`) 端口与打开的地图信息
pub fn discover_running_editor_mcps() -> Vec<Value> {
    let mut listening_map = list_game_exe_listening_ports();
    if auto_rebind_shadowed_editor_mcps(&listening_map) {
        listening_map = list_game_exe_listening_ports();
    }

    let mut candidate_ports: Vec<(u16, Option<u32>)> = Vec::new();
    let mut seen_ports = HashSet::new();

    for (pid, ports) in listening_map {
        for port in ports {
            if is_candidate_mcp_http_port(port) && seen_ports.insert(port) {
                candidate_ports.push((port, Some(pid)));
            }
        }
    }

    // 始终把默认 8765 放入兜底探测列表
    if seen_ports.insert(8765) {
        candidate_ports.push((8765, None));
    }

    let mut editors = Vec::new();
    for (port, pid_opt) in candidate_ports {
        if let Some(info) = probe_editor_mcp_port(port, pid_opt) {
            editors.push(info);
        }
    }
    editors
}

fn probe_editor_mcp_port(port: u16, pid: Option<u32>) -> Option<Value> {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_map_info",
            "arguments": {}
        }
    });
    let resp = forward_jsonrpc_to_editor_mcp_timeout(port, &req, std::time::Duration::from_millis(350)).ok()?;
    let text = resp
        .get("result")?
        .get("content")?
        .as_array()?
        .first()?
        .get("text")?
        .as_str()?;
    let map_info: Value = serde_json::from_str(text).ok()?;
    let project_path = map_info.get("project_path")?.as_str()?.to_string();
    let map_name = map_info
        .get("map_name")
        .and_then(|v| v.as_str())
        .unwrap_or("EntryMap")
        .to_string();
    let width = map_info.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
    let height = map_info.get("height").and_then(|v| v.as_u64()).unwrap_or(0);

    Some(json!({
        "port": port,
        "pid": pid,
        "mcp_url": format!("http://127.0.0.1:{}/mcp", port),
        "project_path": project_path,
        "map_name": map_name,
        "width": width,
        "height": height,
        "server_name": "game_editor_mcp (y3editor)"
    }))
}

fn normalize_path_key(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase()
}

fn resolve_editor_mcp_port(
    target_project_path: Option<&Path>,
    target_map_name: Option<&str>,
) -> anyhow::Result<u16> {
    let editors = discover_running_editor_mcps();
    if editors.is_empty() {
        anyhow::bail!(
            "当前未检测到运行中的 Y3 编辑器 MCP (y3editor)。请先在客户端点击「🛠 打开编辑器」启动对应项目的 Y3 编辑器。"
        );
    }

    if let Some(target_path) = target_project_path {
        let target_norm = normalize_path_key(&target_path.to_string_lossy());
        // 优先同时匹配 project_path 与 map_name
        if let Some(m_name) = target_map_name {
            for ed in &editors {
                let ed_proj = ed.get("project_path").and_then(|v| v.as_str()).unwrap_or("");
                let ed_map = ed.get("map_name").and_then(|v| v.as_str()).unwrap_or("");
                if normalize_path_key(ed_proj) == target_norm
                    && ed_map.eq_ignore_ascii_case(m_name)
                {
                    if let Some(port) = ed.get("port").and_then(|v| v.as_u64()) {
                        return Ok(port as u16);
                    }
                }
            }
        }
        // 其次匹配 project_path
        for ed in &editors {
            let ed_proj = ed.get("project_path").and_then(|v| v.as_str()).unwrap_or("");
            if normalize_path_key(ed_proj) == target_norm {
                if let Some(port) = ed.get("port").and_then(|v| v.as_u64()) {
                    return Ok(port as u16);
                }
            }
        }

        // 若明确指定了目标项目，但在线编辑器列表中没有任何实例匹配该项目，严禁静默回退到其他项目（避免跨项目误操作）
        let online_summary: Vec<String> = editors
            .iter()
            .map(|ed| {
                let p = ed.get("project_path").and_then(|v| v.as_str()).unwrap_or("unknown");
                let port = ed.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                format!("{} (port {})", p, port)
            })
            .collect();
        anyhow::bail!(
            "未找到打开项目 '{}' 的 Y3 编辑器实例（当前在线编辑器：{}）。请先打开目标项目的编辑器。",
            target_path.display(),
            online_summary.join(", ")
        );
    }

    // 未指定目标项目时，回退到第一个在线编辑器实例
    editors
        .first()
        .and_then(|ed| ed.get("port").and_then(|v| v.as_u64()))
        .map(|p| p as u16)
        .ok_or_else(|| anyhow::anyhow!("未找到可用的编辑器 MCP 端口"))
}

/// 向指定本地端口的 Y3 编辑器 `game_editor_mcp` (`/mcp`) 转发 JSON-RPC 请求
pub fn forward_jsonrpc_to_editor_mcp(port: u16, rpc_payload: &Value) -> anyhow::Result<Value> {
    forward_jsonrpc_to_editor_mcp_timeout(port, rpc_payload, std::time::Duration::from_secs(15))
}

pub fn forward_jsonrpc_to_editor_mcp_timeout(
    port: u16,
    rpc_payload: &Value,
    read_timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    use std::io::Read;
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let body = serde_json::to_string(rpc_payload)?;
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    let _ = stream.set_read_timeout(Some(read_timeout));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    let http_req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        port,
        body.len(),
        body
    );
    stream.write_all(http_req.as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    if !status_line.starts_with("HTTP/") {
        anyhow::bail!("非 HTTP 端口");
    }

    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().ok();
            }
        }
    }

    let body_part = if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        String::from_utf8_lossy(&buf).into_owned()
    } else {
        let mut raw_resp = Vec::new();
        let _ = reader.read_to_end(&mut raw_resp);
        String::from_utf8_lossy(&raw_resp).into_owned()
    };

    // 处理可能的 SSE (`data: {...}`) 或直接 JSON
    for line in body_part.lines() {
        let trimmed = line.trim();
        if let Some(sse_json) = trimmed.strip_prefix("data:") {
            if let Ok(v) = serde_json::from_str::<Value>(sse_json.trim()) {
                return Ok(v);
            }
        }
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                return Ok(v);
            }
        }
    }

    let v: Value = serde_json::from_str(body_part.trim())?;
    Ok(v)
}

async fn handle_api_runtime_mcps(State(state): State<HttpAppState>) -> Json<Value> {
    let list = state.dispatcher.discover_running_runtime_mcps();
    Json(json!({
        "count": list.len(),
        "runtimes": list
    }))
}

/// 常驻 `y3runtime` 智能多项目/多窗口路由网关 (`POST /y3runtime/mcp`)
/// 优势：
/// 1. 即使游戏尚未启动或正在重启，网关始终保持在线并正常响应 `initialize` 和 `tools/list`，彻底免除人工手动刷新 `y3runtime` MCP 连接！
/// 2. 调用 `trigger_ui_touch_event_by_path` 时，自动根据 `projectPath`（或当前客户端选中项目）和 `clientSlot` 匹配目标游戏进程的原生 `y3runtime` 端口或通过专属 Lua 通道精准触发 UI 事件。
async fn handle_y3runtime_mcp_proxy(
    State(state): State<HttpAppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    let mut session_id = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let method = match payload.get("method").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => return StatusCode::ACCEPTED.into_response(),
    };
    let req_id = match payload.get("id").cloned() {
        Some(id) => id,
        None => return StatusCode::ACCEPTED.into_response(),
    };
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));

    if method == "initialize" && session_id.is_none() {
        let new_id = uuid::Uuid::new_v4().to_string();
        state.sessions.write().insert(new_id.clone());
        session_id = Some(new_id);
    }

    let reply = match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "y3runtime-gateway",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": "Always-on Y3 Runtime UI Automation MCP Gateway (supports multi-project projectPath and multi-window clientSlot routing without manual reconnection)."
            }
        }),
        "ping" => json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {}
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [
                    {
                        "name": "trigger_ui_touch_event_by_path",
                        "description": "Trigger UI touch/click event by widget path in the running Y3 game window. Supports optional projectPath and clientSlot for multi-project / multi-window concurrent testing.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "ui_path": {
                                    "type": "string",
                                    "description": "UI widget path, e.g. 'panel_1.button_1' or 'WuxiaHUD.quest.button_quest'"
                                },
                                "touch": {
                                    "type": "string",
                                    "description": "Touch button type, e.g. '左键' or '右键'",
                                    "default": "左键"
                                },
                                "projectPath": {
                                    "type": "string",
                                    "description": "Optional target Y3 project path or ID"
                                },
                                "clientSlot": {
                                    "type": "integer",
                                    "description": "Optional target game client window slot (0, 1, ...)"
                                },
                                "runtimePort": {
                                    "type": "integer",
                                    "description": "Optional explicit y3runtime MCP port"
                                }
                            },
                            "required": ["ui_path"]
                        }
                    }
                ]
            }
        }),
        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("trigger_ui_touch_event_by_path");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let header_proj = headers
                .get("x-y3-project")
                .and_then(|v| v.to_str().ok());
            let proj_sel = args
                .get("projectPath")
                .or_else(|| args.get("project_path"))
                .and_then(|v| v.as_str())
                .or(header_proj);
            let slot = args
                .get("clientSlot")
                .or_else(|| args.get("client_slot"))
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let explicit_port = args
                .get("runtimePort")
                .or_else(|| args.get("runtime_port"))
                .and_then(|v| v.as_u64())
                .map(|n| n as u16);
            let ui_path = args
                .get("ui_path")
                .or_else(|| args.get("uiPath"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let touch = args
                .get("touch")
                .and_then(|v| v.as_str())
                .unwrap_or("左键");

            match state
                .dispatcher
                .execute_y3runtime_action(proj_sel, slot, explicit_port, tool_name, ui_path, touch)
                .await
            {
                Ok(_) => json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": "{\"success\": true}"
                        }]
                    }
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": format!("{{\"success\": false, \"error\": {:?}}}", e.to_string())
                        }],
                        "isError": true
                    }
                }),
            }
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {}", method)
            }
        }),
    };

    let mut resp = (StatusCode::OK, Json(reply)).into_response();
    if let Some(sid) = session_id {
        if let Ok(hv) = sid.parse() {
            resp.headers_mut().insert("Mcp-Session-Id", hv);
        }
    }
    resp
}

/// 获取本机所有 `Game_x64h.exe` 进程 PID 及其监听的 TCP 端口映射
fn list_game_exe_listening_ports() -> std::collections::HashMap<u32, Vec<u16>> {
    #[cfg(target_os = "windows")]
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut map: std::collections::HashMap<u32, Vec<u16>> = std::collections::HashMap::new();
    let mut game_pids = HashSet::new();
    let mut cmd_tasklist = Command::new("tasklist");
    cmd_tasklist.args(["/FI", "IMAGENAME eq Game_x64h.exe", "/FO", "CSV", "/NH"]);
    #[cfg(target_os = "windows")]
    cmd_tasklist.creation_flags(CREATE_NO_WINDOW);
    if let Ok(out) = cmd_tasklist.output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let pid_str = parts[1].trim_matches('"').trim();
                if let Ok(pid) = pid_str.parse::<u32>() {
                    game_pids.insert(pid);
                }
            }
        }
    }
    if game_pids.is_empty() {
        return map;
    }
    let mut cmd_netstat = Command::new("netstat");
    cmd_netstat.args(["-ano", "-p", "tcp"]);
    #[cfg(target_os = "windows")]
    cmd_netstat.creation_flags(CREATE_NO_WINDOW);
    if let Ok(out) = cmd_netstat.output() {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 5 && cols[3].eq_ignore_ascii_case("LISTENING") {
                if let Ok(pid) = cols[4].parse::<u32>() {
                    if game_pids.contains(&pid) {
                        if let Some(port_str) = cols[1].rsplit(':').next() {
                            if let Ok(port) = port_str.parse::<u16>() {
                                let entry = map.entry(pid).or_default();
                                if !entry.contains(&port) {
                                    entry.push(port);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    map
}

/// 探测指定端口是否为运行中的 `y3runtime` MCP 服务
fn probe_runtime_mcp_port(port: u16) -> bool {
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "y3-helper-probe", "version": "1.0" }
        }
    });
    if let Ok((resp, _sid)) = send_raw_mcp_http(port, &init_req, None, std::time::Duration::from_millis(250)) {
        if let Some(res) = resp.get("result") {
            let server_name = res
                .get("serverInfo")
                .and_then(|s| s.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            // 只要不是编辑器的 game_editor_mcp，或者端口是 8767，即判定为 y3runtime
            if !server_name.eq_ignore_ascii_case("game_editor_mcp") || port == 8767 {
                return true;
            }
        }
    }
    false
}

/// 带完整 Session 握手 (`initialize` -> `mcp-session-id` -> `notifications/initialized` -> `method`) 调用本地 MCP HTTP 端口
fn call_mcp_http_with_session(port: u16, method: &str, params: Value) -> anyhow::Result<Value> {
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "y3-helper-gateway", "version": "1.0" }
        }
    });
    let (_, sid) = send_raw_mcp_http(port, &init_req, None, std::time::Duration::from_secs(3))?;
    if sid.is_some() {
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let _ = send_raw_mcp_http(port, &notif, sid.as_deref(), std::time::Duration::from_secs(2));
    }
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    });
    let (resp, _) = send_raw_mcp_http(port, &req, sid.as_deref(), std::time::Duration::from_secs(10))?;
    Ok(resp)
}

fn send_raw_mcp_http(
    port: u16,
    rpc_payload: &Value,
    session_id: Option<&str>,
    read_timeout: std::time::Duration,
) -> anyhow::Result<(Value, Option<String>)> {
    use std::io::Read;
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let body = serde_json::to_string(rpc_payload)?;
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    let _ = stream.set_read_timeout(Some(read_timeout));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    let sid_header = session_id
        .map(|s| format!("mcp-session-id: {}\r\n", s))
        .unwrap_or_default();
    let http_req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{}Connection: close\r\nContent-Length: {}\r\n\r\n{}",
        port,
        sid_header,
        body.len(),
        body
    );
    stream.write_all(http_req.as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    if !status_line.starts_with("HTTP/") {
        anyhow::bail!("非 HTTP 端口");
    }

    let mut content_length: Option<usize> = None;
    let mut resp_sid: Option<String> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().ok();
            } else if key.eq_ignore_ascii_case("mcp-session-id") {
                resp_sid = Some(v.trim().to_string());
            }
        }
    }

    if content_length == Some(0) {
        return Ok((json!({}), resp_sid));
    }

    let body_part = if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        String::from_utf8_lossy(&buf).into_owned()
    } else {
        let mut raw_resp = Vec::new();
        let _ = reader.read_to_end(&mut raw_resp);
        String::from_utf8_lossy(&raw_resp).into_owned()
    };

    for line in body_part.lines() {
        let trimmed = line.trim();
        if let Some(sse_json) = trimmed.strip_prefix("data:") {
            if let Ok(v) = serde_json::from_str::<Value>(sse_json.trim()) {
                return Ok((v, resp_sid));
            }
        }
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                return Ok((v, resp_sid));
            }
        }
    }

    let v: Value = serde_json::from_str(body_part.trim()).unwrap_or_else(|_| json!({}));
    Ok((v, resp_sid))
}

/// 生成可在任意项目、任意多开窗口槽位执行的 UI 触摸/点击触发 Lua 代码（兼容 `y3` / `clicli` / `GameAPI`）
fn build_lua_ui_touch_script(ui_path: &str, touch: &str) -> String {
    let is_right = touch.contains("右") || touch.eq_ignore_ascii_case("right");
    let fast_ev = if is_right { "Right-PressDown" } else { "Left-PressDown" };
    format!(
        r#"local __path = {path:?}
local __ev = {ev:?}
local __fw = rawget(_G, 'y3') or rawget(_G, 'clicli')
assert(__fw and __fw.ui, '未找到 y3/clicli UI 框架')
local __player = __fw.player(1)
local __ui = __fw.ui.get_ui(__player, __path)
assert(__ui, '未找到 UI 控件: ' .. tostring(__path))
local __fired = 0
if __fw.trigger and __fw.trigger.all_triggers then
    local __fast_id = string.format('$fast_event:%s@%s', __ev, tostring(__ui.handle))
    for _, t in pairs(__fw.trigger.all_triggers) do
        if t._event and t._event.event_name == 'Interface - Message' then
            local a = t._event_args
            if a == __fast_id or (type(a) == 'table' and a[1] == __fast_id) then
                if t._callback then
                    t:_callback({{ player = __player, ui_event_name = __fast_id, comp_name = __ui.handle }})
                    __fired = __fired + 1
                end
            end
        end
    end
end
return string.format('UI_TOUCH_OK path=%s handle=%s fired=%d', __path, tostring(__ui.handle), __fired)"#,
        path = ui_path,
        ev = fast_ev
    )
}


