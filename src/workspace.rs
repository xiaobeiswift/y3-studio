use crate::console::session::GameSessionManager;
use crate::console::ConsoleServer;
use crate::env::{normalize_path, EditorEnv, ProjectContext};
use crate::features::plugin::PluginManager;
use crate::meta_builder::MetaBuilder;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use parking_lot::{Mutex, RwLock};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 单个已挂载的 Y3 项目运行时工作区（拥有独立的 `ProjectContext`、独立的 `ConsoleServer`、独立的 `GameSessionManager` 与独立的文件监听器）
pub struct ProjectWorkspace {
    pub project_id: String,
    pub project_path: PathBuf,
    pub context: Arc<RwLock<ProjectContext>>,
    pub console_server: Arc<ConsoleServer>,
    pub session_manager: Arc<GameSessionManager>,
    _watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

impl ProjectWorkspace {
    pub async fn new(ctx: ProjectContext, enable_watcher: bool) -> anyhow::Result<Arc<Self>> {
        let project_id = ctx.project_id.clone();
        let project_path = ctx.project_path.clone();

        // 为该项目分配独立的 TCP ConsoleServer 端口
        let console_server = ConsoleServer::start(&project_id, 0).await?;

        // 将端口号写入该项目下所有子地图的 script/log/helper_port.lua
        for map in ctx.maps.values() {
            let _ = console_server.write_helper_port_file(&map.script_path);
        }

        let session_manager = GameSessionManager::new(&project_id, console_server.clone());
        let context = Arc::new(RwLock::new(ctx));

        // 首次构建所有子地图的 Meta
        {
            let guard = context.read();
            let _ = MetaBuilder::build_all_maps(&guard);
        }

        let ws = Arc::new(Self {
            project_id,
            project_path: project_path.clone(),
            context: context.clone(),
            console_server,
            session_manager,
            _watcher: Mutex::new(None),
        });

        if enable_watcher {
            if let Ok(watcher) = Self::setup_watcher(context, &project_path) {
                *ws._watcher.lock() = Some(watcher);
            }
        }

        Ok(ws)
    }

    fn setup_watcher(
        context: Arc<RwLock<ProjectContext>>,
        project_path: &Path,
    ) -> anyhow::Result<notify::RecommendedWatcher> {
        let ctx_clone = context.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if !matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    ) {
                        return;
                    }
                    for path in &event.paths {
                        let fname = path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_lowercase();

                        // 监听自定义定义文件变化，自动重新生成 Meta
                        if matches!(
                            fname.as_str(),
                            "attr.json"
                                | "campinfo.json"
                                | "customevent.json"
                                | "uianim.json"
                                | "jumpword.json"
                                | "font.json"
                                | "sceneui.json"
                        ) {
                            let guard = ctx_clone.read();
                            let _ = MetaBuilder::build_all_maps(&guard);
                        } else if fname.ends_with(".gmp") {
                            // 监听地图保存 (.gmp 修改)，自动触发插件的 onSave
                            let guard = ctx_clone.read();
                            let _ =
                                PluginManager::run_plugin_function(&guard, None, None, "onSave");
                        }
                    }
                }
            })?;

        let maps_dir = project_path.join("maps");
        if maps_dir.is_dir() {
            let _ = watcher.watch(&maps_dir, RecursiveMode::Recursive);
        }
        Ok(watcher)
    }

    pub fn summary_json(&self, is_active: bool) -> Value {
        let ctx = self.context.read();
        let maps_list: Vec<Value> = ctx
            .maps
            .values()
            .map(|m| {
                json!({
                    "id": m.id,
                    "name": m.name,
                    "description": m.description,
                    "map_path": m.map_path,
                    "script_path": m.script_path,
                    "is_entry_map": m.is_entry_map,
                    "is_current_map": m.name == ctx.current_map_name,
                })
            })
            .collect();

        json!({
            "project_id": self.project_id,
            "project_path": self.project_path,
            "is_active": is_active,
            "entry_map_name": ctx.entry_map_name,
            "current_map_name": ctx.current_map_name,
            "use_main_level_trigger_and_object": ctx.use_main_level_trigger_and_object,
            "use_main_level_ui": ctx.use_main_level_ui,
            "console_port": self.console_server.port,
            "connected_clients": self.console_server.list_clients().len(),
            "maps": maps_list,
            "game_status": self.session_manager.get_game_status(),
        })
    }
}

/// 同一台机器多项目、多地图并发管理器
pub struct MultiProjectManager {
    projects: RwLock<BTreeMap<String, Arc<ProjectWorkspace>>>,
    path_to_id: RwLock<BTreeMap<PathBuf, String>>,
    active_project_id: RwLock<Option<String>>,
    editor_env: RwLock<Option<EditorEnv>>,
    enable_watchers: bool,
}

impl MultiProjectManager {
    pub fn new(custom_editor_path: Option<&Path>, enable_watchers: bool) -> Arc<Self> {
        let editor_env = EditorEnv::detect(custom_editor_path);
        Arc::new(Self {
            projects: RwLock::new(BTreeMap::new()),
            path_to_id: RwLock::new(BTreeMap::new()),
            active_project_id: RwLock::new(None),
            editor_env: RwLock::new(editor_env),
            enable_watchers,
        })
    }

    /// 获取或重新探测编辑器环境
    pub fn editor_env(&self) -> Option<EditorEnv> {
        let current = self.editor_env.read().clone();
        if current.is_some() {
            return current;
        }
        let detected = EditorEnv::detect(None);
        if detected.is_some() {
            *self.editor_env.write() = detected.clone();
        }
        detected
    }

    pub fn list_workspaces(&self) -> Vec<Arc<ProjectWorkspace>> {
        self.projects.read().values().cloned().collect()
    }

    /// 手动设置 `Editor.exe` 路径（并自动持久化到本地配置）
    pub fn set_editor_path(&self, editor_exe: &Path) -> anyhow::Result<EditorEnv> {
        let env = EditorEnv::from_editor_exe(editor_exe.to_path_buf())
            .ok_or_else(|| anyhow::anyhow!("无效的 Editor.exe 路径: {}", editor_exe.display()))?;
        EditorEnv::save_editor_path_config(&env.editor_exe_path);
        *self.editor_env.write() = Some(env.clone());
        Ok(env)
    }

    /// 注册或更新一个 Y3 项目（若已存在则复用已有 `ProjectWorkspace` 和 `ConsoleServer` 连接）
    pub async fn register_project(
        &self,
        any_path: &Path,
        set_active: bool,
    ) -> anyhow::Result<Arc<ProjectWorkspace>> {
        let new_ctx = ProjectContext::load_from_path(any_path)?;
        let norm_root = normalize_path(&new_ctx.project_path);

        if let Some(existing_id) = self.path_to_id.read().get(&norm_root).cloned() {
            if let Some(ws) = self.projects.read().get(&existing_id).cloned() {
                // 如果传入的是某个具体子地图路径，更新该项目的默认地图并刷新地图列表
                {
                    let mut guard = ws.context.write();
                    let _ = guard.reload();
                    if guard.maps.contains_key(&new_ctx.current_map_name) {
                        guard.current_map_name = new_ctx.current_map_name;
                    }
                    for map in guard.maps.values() {
                        let _ = ws.console_server.write_helper_port_file(&map.script_path);
                    }
                }
                if set_active {
                    *self.active_project_id.write() = Some(existing_id);
                }
                return Ok(ws);
            }
        }

        let ws = ProjectWorkspace::new(new_ctx, self.enable_watchers).await?;
        let pid = ws.project_id.clone();

        self.projects.write().insert(pid.clone(), ws.clone());
        self.path_to_id.write().insert(norm_root, pid.clone());

        if set_active || self.active_project_id.read().is_none() {
            *self.active_project_id.write() = Some(pid);
        }

        if self.enable_watchers {
            self.save_projects_config();
        }

        Ok(ws)
    }

    fn save_projects_config(&self) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let dir = PathBuf::from(appdata).join("y3-helper");
            let _ = std::fs::create_dir_all(&dir);
            let cfg_path = dir.join("projects.json");
            let paths: Vec<String> = self
                .projects
                .read()
                .values()
                .map(|w| w.project_path.to_string_lossy().to_string())
                .collect();
            let _ = std::fs::write(
                cfg_path,
                serde_json::to_string_pretty(&json!({ "projects": paths })).unwrap_or_default(),
            );
        }
    }

    /// 根据可选选择器（`project_id`、文件夹名或磁盘路径）解析目标项目工作区；
    /// 若传入的是尚未注册的有效磁盘路径，将自动注册并挂载该项目！
    pub async fn resolve_project(
        &self,
        selector: Option<&str>,
    ) -> anyhow::Result<Arc<ProjectWorkspace>> {
        if let Some(sel) = selector.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            // 1. 按 project_id 精确查找
            if let Some(ws) = self.projects.read().get(sel).cloned() {
                return Ok(ws);
            }
            // 2. 按项目目录名或规范化路径查找已注册项目
            {
                let projs = self.projects.read();
                for ws in projs.values() {
                    if ws.project_path.to_string_lossy().eq_ignore_ascii_case(sel)
                        || ws
                            .project_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.eq_ignore_ascii_case(sel))
                            .unwrap_or(false)
                    {
                        return Ok(ws.clone());
                    }
                }
            }
            // 3. 若是磁盘上存在的路径，自动即时挂载该项目（不强制改变当前默认活动项目）
            let p = Path::new(sel);
            if p.exists() {
                return self.register_project(p, false).await;
            }
            anyhow::bail!(
                "未找到项目 '{}'（当前已注册项目：{:?}）",
                sel,
                self.projects.read().keys().collect::<Vec<_>>()
            );
        }

        let active_id = self.active_project_id.read().clone().ok_or_else(|| {
            anyhow::anyhow!(
                "当前尚未绑定任何 Y3 项目，请先调用 set_project_path / register_project 或传入 projectPath 参数"
            )
        })?;

        self.projects
            .read()
            .get(&active_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("活动项目 {} 不存在", active_id))
    }

    /// 切换默认活动项目
    pub fn set_active_project(&self, selector: &str) -> anyhow::Result<Arc<ProjectWorkspace>> {
        let projs = self.projects.read();
        if let Some(ws) = projs.get(selector) {
            *self.active_project_id.write() = Some(ws.project_id.clone());
            return Ok(ws.clone());
        }
        for ws in projs.values() {
            if ws.project_path.to_string_lossy().eq_ignore_ascii_case(selector)
                || ws
                    .project_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.eq_ignore_ascii_case(selector))
                    .unwrap_or(false)
            {
                *self.active_project_id.write() = Some(ws.project_id.clone());
                return Ok(ws.clone());
            }
        }
        anyhow::bail!("未找到已注册的项目 '{}'", selector)
    }

    /// 移除并停止指定项目的工作区
    pub fn remove_project(&self, selector: &str) -> bool {
        let sel_norm = normalize_path(Path::new(selector));
        let target_id = {
            let projs = self.projects.read();
            if projs.contains_key(selector) {
                Some(selector.to_string())
            } else {
                projs
                    .values()
                    .find(|ws| {
                        normalize_path(&ws.project_path) == sel_norm
                            || ws.project_path.to_string_lossy().eq_ignore_ascii_case(selector)
                            || ws
                                .project_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map(|n| n.eq_ignore_ascii_case(selector))
                                .unwrap_or(false)
                    })
                    .map(|ws| ws.project_id.clone())
            }
        };

        if let Some(pid) = target_id {
            let removed_ws = {
                let mut guard = self.projects.write();
                guard.remove(&pid)
            };
            if let Some(ws) = removed_ws {
                ws.console_server.stop();
                let norm = normalize_path(&ws.project_path);
                self.path_to_id.write().remove(&norm);
                let next_id = self.projects.read().keys().next().cloned();
                {
                    let mut active = self.active_project_id.write();
                    if active.as_deref() == Some(&pid) {
                        *active = next_id;
                    }
                }
                if self.enable_watchers {
                    self.save_projects_config();
                }
                return true;
            }
        }
        false
    }

    /// 列出所有已挂载的项目及其子地图和游戏状态
    pub fn list_projects_json(&self) -> Value {
        let active_id = self.active_project_id.read().clone();
        let projs = self.projects.read();
        let list: Vec<Value> = projs
            .values()
            .map(|ws| ws.summary_json(active_id.as_deref() == Some(&ws.project_id)))
            .collect();

        json!({
            "active_project_id": active_id,
            "total_projects": list.len(),
            "editor": self.editor_env(),
            "projects": list,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_multi_project_concurrent_operations() {
        let tmp = tempfile::tempdir().unwrap();

        // 创建项目 A（含 EntryMap 和 ArenaMap 两个子地图）
        let proj_a = tmp.path().join("ProjectAlpha");
        fs::create_dir_all(proj_a.join("maps/EntryMap/script")).unwrap();
        fs::create_dir_all(proj_a.join("maps/ArenaMap/script")).unwrap();
        fs::write(
            proj_a.join("header.project"),
            r#"{"entry_map": {"id": 101}}"#,
        )
        .unwrap();
        fs::write(proj_a.join("maps/EntryMap/header.map"), r#"{"id": 101}"#).unwrap();
        fs::write(proj_a.join("maps/ArenaMap/header.map"), r#"{"id": 102}"#).unwrap();

        // 创建项目 B（含 EntryMap 和 BossMap 两个子地图）
        let proj_b = tmp.path().join("ProjectBeta");
        fs::create_dir_all(proj_b.join("maps/EntryMap/script")).unwrap();
        fs::create_dir_all(proj_b.join("maps/BossMap/script")).unwrap();
        fs::write(
            proj_b.join("header.project"),
            r#"{"entry_map": {"id": 201}}"#,
        )
        .unwrap();
        fs::write(proj_b.join("maps/EntryMap/header.map"), r#"{"id": 201}"#).unwrap();
        fs::write(proj_b.join("maps/BossMap/header.map"), r#"{"id": 202}"#).unwrap();

        let mgr = MultiProjectManager::new(None, false);
        let ws_a = mgr.register_project(&proj_a, true).await.unwrap();
        let ws_b = mgr.register_project(&proj_b, false).await.unwrap();

        // 验证两个项目拥有各自独立的 ConsoleServer 端口
        assert_ne!(ws_a.console_server.port, ws_b.console_server.port);

        // 验证各自的 helper_port.lua 写入了各自的端口
        let port_a_file =
            fs::read_to_string(proj_a.join("maps/ArenaMap/script/log/helper_port.lua")).unwrap();
        let port_b_file =
            fs::read_to_string(proj_b.join("maps/BossMap/script/log/helper_port.lua")).unwrap();
        assert_eq!(port_a_file, format!("return {}", ws_a.console_server.port));
        assert_eq!(port_b_file, format!("return {}", ws_b.console_server.port));

        // 验证可以按项目名/ID 同时检索并操作
        let resolved_a = mgr.resolve_project(Some("ProjectAlpha")).await.unwrap();
        let resolved_b = mgr.resolve_project(Some(&ws_b.project_id)).await.unwrap();
        assert_eq!(resolved_a.project_id, ws_a.project_id);
        assert_eq!(resolved_b.project_id, ws_b.project_id);
    }
}
