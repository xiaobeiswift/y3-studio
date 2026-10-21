use crate::console::{ConsoleClient, ConsoleServer};
use crate::env::{EditorEnv, ProjectContext};
use crate::launcher::{GameLauncher, LaunchGameOptions};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub id: String,
    pub status: String, // "launching" | "running" | "restarting" | "stopped"
    pub start_time_ms: i64,
    pub map_name: String,
    pub launched_pids: Vec<u32>,
    pub error_message: Option<String>,
}

/// 每个项目独立的游戏会话管理器（对应原版 `GameSessionManager`，但按项目隔离）
pub struct GameSessionManager {
    pub project_id: String,
    pub console_server: Arc<ConsoleServer>,
    current_session: Arc<RwLock<Option<SessionState>>>,
}

impl GameSessionManager {
    pub fn new(project_id: &str, console_server: Arc<ConsoleServer>) -> Arc<Self> {
        Arc::new(Self {
            project_id: project_id.to_string(),
            console_server,
            current_session: Arc::new(RwLock::new(None)),
        })
    }

    pub fn record_launched_pids(&self, pids: Vec<u32>, map_name: &str) -> String {
        let now = chrono::Utc::now().timestamp_millis();
        let session_id = format!("session_{}_{}", self.project_id, now);
        *self.current_session.write() = Some(SessionState {
            id: session_id.clone(),
            status: "launching".to_string(),
            start_time_ms: now,
            map_name: map_name.to_string(),
            launched_pids: pids,
            error_message: None,
        });
        session_id
    }

    pub fn known_pids(&self) -> Vec<u32> {
        self.current_session
            .read()
            .as_ref()
            .map(|s| s.launched_pids.clone())
            .unwrap_or_default()
    }

    /// 启动游戏并自动监控客户端连接（兼容原版 `launchGame` 返回格式）
    pub async fn launch_game(
        &self,
        project: &ProjectContext,
        editor: Option<&EditorEnv>,
        options: LaunchGameOptions,
    ) -> Value {
        let existing_clients = self.console_server.list_clients();
        if !existing_clients.is_empty() && !options.multi_mode {
            let sid = self
                .current_session
                .read()
                .as_ref()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| format!("session_{}", chrono::Utc::now().timestamp_millis()));
            return json!({
                "success": true,
                "project_id": project.project_id,
                "session_id": sid,
                "status": "running",
                "console_port": self.console_server.port,
                "message": "Game is already running"
            });
        }

        let ed = match editor {
            Some(e) => e.clone(),
            None => match EditorEnv::detect(None) {
                Some(e) => e,
                None => {
                    return json!({
                        "success": false,
                        "status": "stopped",
                        "error": "未找到 Y3 编辑器路径 (Editor.exe)，请通过 set_editor_path 或环境变量 Y3_EDITOR_PATH 指定"
                    });
                }
            },
        };

        match GameLauncher::launch_game(project, &ed, &self.console_server, &options) {
            Ok(res) => {
                let pids: Vec<u32> = res.processes.iter().map(|p| p.pid).collect();
                let session_id = self.record_launched_pids(pids.clone(), &res.map_name);

                // 后台协程等待客户端连接并将状态切换为 running
                let srv = self.console_server.clone();
                let sess_lock = self.current_session.clone();
                let sid_clone = session_id.clone();
                tokio::spawn(async move {
                    let start = std::time::Instant::now();
                    while start.elapsed() < std::time::Duration::from_secs(110) {
                        if !srv.list_clients().is_empty() {
                            if let Some(ref mut s) = *sess_lock.write() {
                                if s.id == sid_clone {
                                    s.status = "running".to_string();
                                }
                            }
                            return;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                    if let Some(ref mut s) = *sess_lock.write() {
                        if s.id == sid_clone && s.status == "launching" {
                            s.status = "stopped".to_string();
                            s.error_message = Some(
                                "Game launched but client connection timeout (waited 110 seconds)"
                                    .to_string(),
                            );
                        }
                    }
                });

                json!({
                    "success": true,
                    "project_id": project.project_id,
                    "map_name": res.map_name,
                    "session_id": session_id,
                    "console_port": res.console_port,
                    "processes": res.processes,
                    "status": "launching",
                    "message": "Game is launching. Use get_game_status to check when it is ready."
                })
            }
            Err(e) => json!({
                "success": false,
                "project_id": project.project_id,
                "status": "stopped",
                "error": e.to_string()
            }),
        }
    }

    /// 获取当前项目的游戏运行状态（兼容原版 `getGameStatus`）
    pub fn get_game_status(&self) -> Value {
        let client_list = self.console_server.list_clients();
        let has_connected = !client_list.is_empty();
        let clients_json: Vec<Value> = client_list
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "slot": c.slot,
                    "name": c.name(),
                    "role_name": c.name(),
                    "multi_mode": c.is_multi_mode(),
                    "log_count": c.log_manager.total_lines()
                })
            })
            .collect();

        let mut sess_guard = self.current_session.write();
        if let Some(ref mut sess) = *sess_guard {
            if has_connected && (sess.status == "launching" || sess.status == "restarting") {
                sess.status = "running".to_string();
            } else if !has_connected && sess.status == "running" {
                sess.status = "stopped".to_string();
            }

            let now = chrono::Utc::now().timestamp_millis();
            return json!({
                "project_id": self.project_id,
                "console_port": self.console_server.port,
                "running": sess.status == "running" || has_connected,
                "session_id": sess.id,
                "map_name": sess.map_name,
                "status": sess.status,
                "uptime": (now - sess.start_time_ms).max(0),
                "client_connected": has_connected,
                "clients": clients_json,
                "error": sess.error_message,
            });
        }

        if has_connected {
            json!({
                "project_id": self.project_id,
                "console_port": self.console_server.port,
                "running": true,
                "session_id": null,
                "status": "running",
                "message": "Game is running (connected client detected)",
                "clients": clients_json
            })
        } else {
            json!({
                "project_id": self.project_id,
                "console_port": self.console_server.port,
                "running": false,
                "session_id": null,
                "status": "no_session",
                "clients": clients_json
            })
        }
    }

    fn resolve_client(&self, client_slot: Option<usize>) -> anyhow::Result<Arc<ConsoleClient>> {
        let clients = self.console_server.list_clients();
        if let Some(slot) = client_slot {
            clients
                .into_iter()
                .find(|c| c.slot == slot)
                .ok_or_else(|| anyhow::anyhow!("Client slot {} not found", slot))
        } else {
            clients
                .last()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("游戏客户端未连接"))
        }
    }

    /// 读取游戏窗口日志（优先读取在线客户端实时内存日志；若客户端未连接或已退出，自动回退读取磁盘上最新的 lua_player*.log 或会话日志）
    pub fn get_logs(
        &self,
        limit: Option<usize>,
        client_slot: Option<usize>,
        script_dir: Option<&std::path::Path>,
    ) -> Value {
        let lim = limit.unwrap_or(100);
        if let Ok(client) = self.resolve_client(client_slot) {
            let mut logs = client.log_manager.read_logs(lim);
            // 若刚连上或内存日志较少，检查磁盘 lua_player*.log 是否包含更完整的启动期日志
            if let Some(sdir) = script_dir {
                if let Some((disk_path, disk_lines)) = Self::read_latest_disk_log(sdir, &self.project_id, lim) {
                    if disk_lines.len() > logs.len() {
                        logs = disk_lines;
                        return json!({
                            "success": true,
                            "online": true,
                            "source": "disk_and_live",
                            "log_file": disk_path,
                            "project_id": self.project_id,
                            "client_slot": client.slot,
                            "client_name": client.name(),
                            "client_count": self.console_server.list_clients().len(),
                            "log_count": logs.len(),
                            "logs": logs.join("\n")
                        });
                    }
                }
            }
            return json!({
                "success": true,
                "online": true,
                "source": "live_console",
                "log_file": client.log_manager.log_file_path,
                "project_id": self.project_id,
                "client_slot": client.slot,
                "client_name": client.name(),
                "client_count": self.console_server.list_clients().len(),
                "log_count": logs.len(),
                "logs": logs.join("\n")
            });
        }

        // 离线回退：自动查找当前子地图 script/.log/、script/log/ 或 %TEMP%/y3-helper-logs 下最新的日志文件
        if let Some(sdir) = script_dir {
            if let Some((disk_path, disk_lines)) = Self::read_latest_disk_log(sdir, &self.project_id, lim) {
                return json!({
                    "success": true,
                    "online": false,
                    "source": "offline_disk_log",
                    "log_file": disk_path,
                    "project_id": self.project_id,
                    "client_count": 0,
                    "log_count": disk_lines.len(),
                    "logs": disk_lines.join("\n")
                });
            }
        }

        json!({
            "success": false,
            "online": false,
            "project_id": self.project_id,
            "message": "游戏客户端未连接，且未找到历史日志文件"
        })
    }

    fn read_latest_disk_log(
        script_dir: &std::path::Path,
        project_id: &str,
        limit: usize,
    ) -> Option<(PathBuf, Vec<String>)> {
        let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        for sub in [".log", "log"] {
            let dir = script_dir.join(sub);
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if name.starts_with("lua_player") && name.ends_with(".log") {
                        if let Ok(meta) = entry.metadata() {
                            if meta.len() > 0 {
                                if let Ok(mtime) = meta.modified() {
                                    candidates.push((p, mtime));
                                }
                            }
                        }
                    }
                }
            }
        }

        let temp_log_dir = std::env::temp_dir().join("y3-helper-logs");
        let safe_prefix: String = project_id
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        let session_prefix = format!("session-{}_", safe_prefix);
        if let Ok(entries) = std::fs::read_dir(&temp_log_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.starts_with(&session_prefix) && name.ends_with(".log") {
                    if let Ok(meta) = entry.metadata() {
                        if meta.len() > 0 {
                            if let Ok(mtime) = meta.modified() {
                                candidates.push((p, mtime));
                            }
                        }
                    }
                }
            }
        }

        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        let (best_path, _) = candidates.into_iter().next()?;
        let bytes = std::fs::read(&best_path).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let all_lines: Vec<String> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        let len = all_lines.len();
        let start = len.saturating_sub(limit);
        Some((best_path, all_lines[start..].to_vec()))
    }

    /// 在运行中的游戏窗口内执行 Lua 代码并捕获新增输出
    pub async fn execute_lua(&self, code: &str, client_slot: Option<usize>) -> Value {
        if code.trim().is_empty() {
            return json!({
                "success": false,
                "project_id": self.project_id,
                "message": "No code provided"
            });
        }
        let client = match self.resolve_client(client_slot) {
            Ok(c) => c,
            Err(e) => {
                return json!({
                    "success": false,
                    "project_id": self.project_id,
                    "message": e.to_string()
                })
            }
        };

        let before_count = client.log_manager.total_lines();
        client.notify("command", json!({ "data": code }));

        tokio::time::sleep(std::time::Duration::from_millis(800)).await;

        let after_all = client.log_manager.read_logs(10000);
        let new_logs = if after_all.len() > before_count {
            &after_all[before_count..]
        } else {
            &[][..]
        };

        json!({
            "success": true,
            "project_id": self.project_id,
            "client_slot": client.slot,
            "client_name": client.name(),
            "output": new_logs.join("\n")
        })
    }

    /// 快速重启游戏窗口（发送 `.rr` 命令并等待重连）
    pub async fn quick_restart(&self, client_slot: Option<usize>) -> Value {
        let old_client = match self.resolve_client(client_slot) {
            Ok(c) => c,
            Err(e) => {
                return json!({
                    "success": false,
                    "project_id": self.project_id,
                    "message": e.to_string()
                })
            }
        };
        let old_id = old_client.id;
        let known_ids: std::collections::HashSet<u64> = self
            .console_server
            .list_clients()
            .into_iter()
            .map(|c| c.id)
            .collect();

        if let Some(ref mut s) = *self.current_session.write() {
            s.status = "restarting".to_string();
        }

        old_client.notify("command", json!({ "data": ".rr" }));

        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(60) {
            let current = self.console_server.list_clients();
            // 检查是否有新重连上的客户端
            if let Some(new_c) = current.iter().find(|c| !known_ids.contains(&c.id)) {
                if let Some(ref mut s) = *self.current_session.write() {
                    s.status = "running".to_string();
                }
                let logs = new_c.log_manager.read_logs(1000);
                return json!({
                    "success": true,
                    "project_id": self.project_id,
                    "client_slot": new_c.slot,
                    "client_name": new_c.name(),
                    "message": "Game restarted successfully",
                    "output": logs.join("\n")
                });
            }
            let _ = old_id;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        json!({
            "success": false,
            "project_id": self.project_id,
            "message": "Game restart timeout - client did not reconnect within 60 seconds"
        })
    }

    /// 停止运行中的游戏（优雅调用 `GameAPI.role_force_quit` + 关闭连接）
    pub async fn stop_game(&self, client_slot: Option<usize>) -> Value {
        if let Some(slot) = client_slot {
            let client = match self.resolve_client(Some(slot)) {
                Ok(c) => c,
                Err(e) => {
                    return json!({
                        "success": false,
                        "project_id": self.project_id,
                        "message": e.to_string()
                    })
                }
            };
            Self::force_quit_client(&client).await;
            return json!({
                "success": true,
                "project_id": self.project_id,
                "client_slot": client.slot,
                "client_name": client.name(),
                "message": "Game stopped"
            });
        }

        for c in self.console_server.list_clients() {
            Self::force_quit_client(&c).await;
        }
        *self.current_session.write() = None;

        json!({
            "success": true,
            "project_id": self.project_id,
            "message": "Game stopped"
        })
    }

    async fn force_quit_client(client: &ConsoleClient) {
        let lua_code = r#"
            local fw = rawget(_G, 'y3') or rawget(_G, 'clicli')
            local player = fw and fw.player and fw.player.get_local and fw.player.get_local()
            if player and player.handle then
                GameAPI.role_force_quit(player.handle, '游戏已停止')
            elseif GameAPI.get_client_role then
                GameAPI.role_force_quit(GameAPI.get_client_role(), '游戏已停止')
            end
        "#;
        client.notify("command", json!({ "data": lua_code }));
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        client.close();
    }

    /// 捕获运行中游戏窗口的截图（与原版 `captureScreenshot` 完全一致，并附带 base64_png 供桌面客户端直接预览）
    pub async fn capture_screenshot(
        &self,
        editor: Option<&EditorEnv>,
        client_slot: Option<usize>,
    ) -> Value {
        let client = match self.resolve_client(client_slot) {
            Ok(c) => c,
            Err(e) => {
                return json!({
                    "success": false,
                    "project_id": self.project_id,
                    "error": e.to_string(),
                    "message": e.to_string()
                })
            }
        };

        let ed = match editor.cloned().or_else(|| EditorEnv::detect(None)) {
            Some(e) => e,
            None => {
                return json!({
                    "success": false,
                    "project_id": self.project_id,
                    "error": "编辑器路径未找到",
                    "message": "编辑器路径未找到"
                })
            }
        };

        let lua_code = r#"
            local width = GameAPI.get_game_x_resolution()
            local height = GameAPI.get_game_y_resolution()
            GameAPI.screenshot_func_for_lua("mcp_screenshots", "screenshot", width, height)
        "#;
        client.notify("command", json!({ "data": lua_code }));

        let screenshot_path = ed
            .editor_exe_path
            .parent()
            .map(|p| {
                let c1 = p.join("LocalData/mcp_screenshots/screenshot.png");
                if c1.parent().map(|d| d.exists()).unwrap_or(false) || p.join("LocalData").is_dir() {
                    c1
                } else {
                    p.parent()
                        .map(|pp| pp.join("LocalData/mcp_screenshots/screenshot.png"))
                        .unwrap_or(c1)
                }
            })
            .unwrap_or_else(|| PathBuf::from("screenshot.png"));

        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        let base64_png = std::fs::read(&screenshot_path).ok().map(|bytes| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(bytes)
        });

        json!({
            "success": true,
            "project_id": self.project_id,
            "client_slot": client.slot,
            "client_name": client.name(),
            "screenshot_path": screenshot_path,
            "file_path": screenshot_path,
            "base64_png": base64_png,
            "message": "截图指令已发送至游戏客户端"
        })
    }

    /// 独立静态诊断项目中的 Lua 文件（无需依赖 VS Code 扩展）
    pub fn read_problems_lua(project: &ProjectContext, path_glob: Option<&Value>) -> Value {
        let mut items = Vec::new();
        let filter_patterns: Vec<String> = match path_glob {
            Some(Value::String(s)) if !s.trim().is_empty() => vec![s.clone()],
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };

        for entry in WalkDir::new(&project.project_path).into_iter().flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("lua") {
                continue;
            }
            let rel = path
                .strip_prefix(&project.project_path)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            // 跳过 y3-lualib 内核文件
            if rel.contains("/script/y3/") || rel.starts_with("global_script/y3/") {
                continue;
            }

            if !filter_patterns.is_empty() {
                let matched = filter_patterns.iter().any(|pat| {
                    let clean = pat.trim_end_matches("/**/*.lua").trim_end_matches("/*.lua");
                    rel.starts_with(clean) || rel.contains(clean)
                });
                if !matched {
                    continue;
                }
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                for (line_idx, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("--") {
                        continue;
                    }
                    // 检查常见 Y3 键盘常量错误: KeyboardKey['1'] -> KeyboardKey['KEY_1']
                    if trimmed.contains("KeyboardKey['") || trimmed.contains("KeyboardKey[\"") {
                        for d in 0..=9 {
                            let bad1 = format!("KeyboardKey['{}']", d);
                            let bad2 = format!("KeyboardKey[\"{}\"]", d);
                            if trimmed.contains(&bad1) || trimmed.contains(&bad2) {
                                items.push(json!({
                                    "file": rel,
                                    "line": line_idx + 1,
                                    "severity": "warning",
                                    "message": format!("键盘数字键常量应使用 y3.const.KeyboardKey['KEY_{}'] 而非 '{}'", d, d)
                                }));
                            }
                        }
                    }
                }
            }
        }

        let total = items.len();
        json!({
            "success": true,
            "project_id": project.project_id,
            "filter": {
                "minSeverity": "warning",
                "pathGlob": path_glob
            },
            "total": total,
            "returned": total,
            "items": items
        })
    }
}
