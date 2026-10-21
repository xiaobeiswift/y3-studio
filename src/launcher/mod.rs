use crate::console::ConsoleServer;
use crate::env::{EditorEnv, ProjectContext};
use crate::gmp::{save_gmp, GmpSaveOptions};
use crate::utils::y3json::convert_tuple_format;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LaunchGameOptions {
    /// 指定要启动的子地图名（若为空则启动项目当前的默认子地图）
    pub map_name: Option<String>,
    /// 是否开启多开模式
    #[serde(default)]
    pub multi_mode: bool,
    /// 多开模式下指定启动的玩家 ID 列表（如 `[1, 2]`；若为空且 `multi_mode=true` 则默认取前 2 个可用角色或 `[1, 2]`）
    pub multi_players: Option<Vec<u32>>,
    /// 是否启动 Tracy 性能分析器
    #[serde(default)]
    pub tracy: bool,
    /// 附加 Lua 启动参数
    #[serde(default)]
    pub lua_args: BTreeMap<String, String>,
    /// 启动前是否自动保存/打包 GMP（默认 true）
    pub save_gmp_before_launch: Option<bool>,
}

use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchedProcessInfo {
    pub pid: u32,
    pub player_id: Option<u32>,
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchGameResult {
    pub project_id: String,
    pub map_name: String,
    pub map_id: u64,
    pub console_port: u16,
    pub multi_mode: bool,
    pub processes: Vec<LaunchedProcessInfo>,
}

pub struct GameLauncher;

impl GameLauncher {
    /// 读取指定项目的可用玩家角色列表（排除中立敌对 31 和中立友善 32，与 `multiPlayerRoles.ts` 一致）
    pub fn load_available_multi_player_ids(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> Vec<u32> {
        let trigger_map = match project.trigger_map_for(map_name) {
            Ok(m) => m,
            Err(_) => return vec![1, 2],
        };
        let mut camp_path = trigger_map.map_path.join("campinfo.json");
        if !camp_path.is_file() {
            if let Ok(entry) = project.entry_map() {
                let fallback = entry.map_path.join("campinfo.json");
                if fallback.is_file() {
                    camp_path = fallback;
                }
            }
        }
        if !camp_path.is_file() {
            return vec![1, 2];
        }
        let text = match fs::read_to_string(&camp_path) {
            Ok(t) => t,
            Err(_) => return vec![1, 2],
        };
        let raw: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return vec![1, 2],
        };
        let plain = convert_tuple_format(&raw);
        let mut ids = BTreeSet::new();
        if let Some(roles) = plain.get("roles").and_then(|v| v.as_array()) {
            for role in roles {
                let obj = role.as_array().and_then(|a| a.first()).unwrap_or(role);
                if let Some(aid) = obj.get("aid").and_then(|v| v.as_u64()) {
                    let pid = aid as u32;
                    if pid >= 1 && pid != 31 && pid != 32 {
                        ids.insert(pid);
                    }
                }
            }
        }
        if ids.is_empty() {
            vec![1, 2]
        } else {
            ids.into_iter().collect()
        }
    }

    /// 准备并同步多开玩家的本地存档昵称（`{entry_map}/archive/archive_storage.json`，与 `multiPlayerArchives.ts` 一致）
    pub fn prepare_multi_player_nicknames(
        project: &ProjectContext,
        player_ids: &[u32],
        editor_nickname: Option<&str>,
    ) -> anyhow::Result<BTreeMap<u32, String>> {
        let entry_map = project.entry_map()?;
        let archive_dir = entry_map.map_path.join("archive");
        let archive_path = archive_dir.join("archive_storage.json");

        let mut storage_obj: Map<String, Value> = if archive_path.is_file() {
            fs::read_to_string(&archive_path)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default()
        } else {
            Map::new()
        };

        let existing_names: Vec<String> = storage_obj
            .keys()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let primary = editor_nickname
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| existing_names.first().cloned())
            .unwrap_or_else(|| "Player 1".to_string());

        let mut used = BTreeSet::new();
        used.insert(primary.clone());

        let mut sorted_pids = player_ids.to_vec();
        sorted_pids.sort_unstable();
        sorted_pids.dedup();

        let mut result = BTreeMap::new();
        let mut storage_changed = false;

        if !storage_obj.contains_key(&primary) {
            storage_obj.insert(primary.clone(), json!({}));
            storage_changed = true;
        }

        for (idx, &pid) in sorted_pids.iter().enumerate() {
            if pid == 1 || idx == 0 {
                result.insert(pid, primary.clone());
                continue;
            }
            let cand = existing_names
                .iter()
                .find(|n| !used.contains(*n))
                .cloned()
                .unwrap_or_else(|| {
                    let mut n = format!("Player {}", pid);
                    let mut suffix = 1;
                    while used.contains(&n) {
                        n = format!("Player {}_{}", pid, suffix);
                        suffix += 1;
                    }
                    n
                });
            used.insert(cand.clone());
            if !storage_obj.contains_key(&cand) {
                storage_obj.insert(cand.clone(), json!({}));
                storage_changed = true;
            }
            result.insert(pid, cand);
        }

        if storage_changed {
            fs::create_dir_all(&archive_dir)?;
            let serialized = serde_json::to_string_pretty(&Value::Object(storage_obj))?;
            fs::write(&archive_path, serialized)?;
        }

        Ok(result)
    }

    /// 启动指定项目、指定子地图的游戏客户端（单人或多开）
    pub fn launch_game(
        project: &ProjectContext,
        editor: &EditorEnv,
        console_server: &ConsoleServer,
        options: &LaunchGameOptions,
    ) -> anyhow::Result<LaunchGameResult> {
        if !editor.game_exe_path.is_file() {
            anyhow::bail!(
                "未找到游戏可执行文件 Game_x64h.exe: {}",
                editor.game_exe_path.display()
            );
        }

        let map = project.resolve_map(options.map_name.as_deref())?;
        let trigger_map = project.trigger_map_for(Some(&map.name))?;

        // 1. 启动前可选保存 GMP
        if options.save_gmp_before_launch.unwrap_or(true) {
            let _ = save_gmp(&trigger_map.map_path, GmpSaveOptions::default());
        }

        // 2. 将当前项目的独立 ConsoleServer 端口写入子地图和主地图的 script/log/helper_port.lua
        console_server.write_helper_port_file(&map.script_path)?;
        if let Ok(entry) = project.entry_map() {
            let _ = console_server.write_helper_port_file(&entry.script_path);
        }

        let wd = editor
            .game_exe_path
            .parent()
            .unwrap_or_else(|| Path::new("."));

        // 3. 可选启动 Tracy
        if options.tracy {
            let tracy_exe = editor
                .editor_exe_path
                .parent()
                .map(|p| p.join("Plugin/Tracy.exe"));
            if let Some(tp) = tracy_exe.filter(|p| p.is_file()) {
                let _ = Command::new(tp)
                    .args(["-a", "127.0.0.1"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
        }

        let mut processes = Vec::new();

        if options.multi_mode {
            let available = Self::load_available_multi_player_ids(project, Some(&map.name));
            let players: Vec<u32> = match &options.multi_players {
                Some(list) if !list.is_empty() => list
                    .iter()
                    .copied()
                    .filter(|id| available.contains(id))
                    .collect(),
                _ => available.into_iter().take(2).collect(),
            };
            if players.is_empty() {
                anyhow::bail!("没有可用的多开玩家角色，请检查 campinfo.json");
            }

            let nicknames = Self::prepare_multi_player_nicknames(
                project,
                &players,
                editor.nickname.as_deref(),
            )?;

            let role_ids_str = players
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join("#");

            let mut py_args = vec![
                "type@editor_game".to_string(),
                "subtype@editor_multi_game".to_string(),
                format!("role_ids@{}", role_ids_str),
                format!("editor_map_path@{}", project.project_path.display()),
                format!("level_id@{}", map.id),
                "release@true".to_string(),
                "lua_dummy@sp ce".to_string(),
            ];
            for (k, v) in &options.lua_args {
                py_args.push(format!("{}@{}", k, v));
            }
            if options.tracy {
                py_args.push("tracy@true".to_string());
            }

            let args = vec![
                "--dx11".to_string(),
                "--start=Python".to_string(),
                format!("--python-args={}", py_args.join(",")),
                "--plugin-config=Plugins-PyQt".to_string(),
            ];

            let pid = Self::spawn_elevated_or_direct(&editor.game_exe_path, &args, wd)?;
            for &p_id in &players {
                processes.push(LaunchedProcessInfo {
                    pid,
                    player_id: Some(p_id),
                    nickname: nicknames.get(&p_id).cloned(),
                });
            }
        } else {
            let mut py_args = vec![
                "type@editor_game".to_string(),
                "subtype@editor_game".to_string(),
                format!("editor_map_path@{}", project.project_path.display()),
                format!("level_id@{}", map.id),
                "release@true".to_string(),
                "lua_dummy@sp ce".to_string(),
            ];
            for (k, v) in &options.lua_args {
                py_args.push(format!("{}@{}", k, v));
            }
            if options.tracy {
                py_args.push("tracy@true".to_string());
            }

            let args = vec![
                "--dx11".to_string(),
                "--start=Python".to_string(),
                format!("--python-args={}", py_args.join(",")),
                "--plugin-config=Plugins-PyQt".to_string(),
                "--console".to_string(),
                "--luaconsole".to_string(),
            ];

            let pid = Self::spawn_elevated_or_direct(&editor.game_exe_path, &args, wd)?;

            processes.push(LaunchedProcessInfo {
                pid,
                player_id: Some(1),
                nickname: editor.nickname.clone(),
            });
        }

        Ok(LaunchGameResult {
            project_id: project.project_id.clone(),
            map_name: map.name.clone(),
            map_id: map.id,
            console_port: console_server.port,
            multi_mode: options.multi_mode,
            processes,
        })
    }

    /// 在 Y3 编辑器中打开指定项目（与 `launchEditor.ts` 完全一致，自动处理 UAC 提权）
    pub fn launch_editor(project: &ProjectContext, editor: &EditorEnv) -> anyhow::Result<u32> {
        if !editor.game_exe_path.is_file() {
            anyhow::bail!(
                "未找到编辑器引擎可执行文件: {}",
                editor.game_exe_path.display()
            );
        }
        let header_path_slash = project
            .header_project_path
            .to_string_lossy()
            .replace('\\', "/");
        let b64 = base64::engine::general_purpose::STANDARD.encode(header_path_slash.as_bytes());
        let py_args = format!(
            "type@editor,subtype@author,release@true,project_path@{},lua_dummy@sp ce",
            b64
        );
        let wd = editor
            .game_exe_path
            .parent()
            .unwrap_or_else(|| Path::new("."));

        let args = vec![
            "--dx11".to_string(),
            "--console".to_string(),
            "--start=Python".to_string(),
            format!("--python-args={}", py_args),
            "--plugin-config=Plugins-PyQt".to_string(),
        ];

        Self::spawn_elevated_or_direct(&editor.game_exe_path, &args, wd)
    }

    /// 优先直接启动进程；若遇到 Windows OS Error 740 (`ERROR_ELEVATION_REQUIRED` 请求的操作需要提升)，
    /// 自动通过内置常驻管理员提权代理（只需授权一次 UAC，后续永久免弹窗）或 PowerShell `RunAs` 启动
    pub fn spawn_elevated_or_direct(
        exe: &Path,
        args: &[String],
        cwd: &Path,
    ) -> anyhow::Result<u32> {
        match Command::new(exe)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => return Ok(child.id()),
            Err(e) if e.raw_os_error() == Some(740) => {
                // 需要管理员权限提升 (os error 740)
            }
            Err(e) => return Err(e.into()),
        }

        // 1. 尝试通过常驻管理员提权代理启动（若代理未运行则拉起它）
        if let Ok(pid) = Self::launch_via_elevated_agent(exe, args, cwd) {
            return Ok(pid);
        }

        // 2. 兜底：直接通过带 UTF-8 BOM 的 PowerShell RunAs 脚本启动目标进程
        Self::launch_via_powershell_runas(exe, args, cwd)
    }

    fn agent_state_file() -> PathBuf {
        std::env::temp_dir().join("y3-helper-elevated-agent.json")
    }

    fn try_request_agent(req: &Value) -> Option<Value> {
        use std::net::TcpStream;
        use std::time::Duration;

        let state_text = fs::read_to_string(Self::agent_state_file()).ok()?;
        let state: Value = serde_json::from_str(&state_text).ok()?;
        let port = state.get("port")?.as_u64()? as u16;
        let token = state.get("token")?.as_str()?;

        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(800)).ok()?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

        let mut payload = req.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("token".to_string(), Value::String(token.to_string()));
        }
        let line = format!("{}\n", serde_json::to_string(&payload).ok()?);
        stream.write_all(line.as_bytes()).ok()?;
        stream.flush().ok()?;

        let mut reader = BufReader::new(stream);
        let mut resp_line = String::new();
        reader.read_line(&mut resp_line).ok()?;
        serde_json::from_str(resp_line.trim()).ok()
    }

    fn ensure_elevated_agent_running(allowed_exe: &Path) -> anyhow::Result<()> {
        if let Some(resp) = Self::try_request_agent(&json!({ "type": "ping" })) {
            if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                return Ok(());
            }
        }

        let self_exe = std::env::current_exe()?;
        let launcher_ps1 = std::env::temp_dir().join("y3-helper-elevate-agent.ps1");
        let quote_ps = |s: &str| format!("'{}'", s.replace('\'', "''"));

        let script = format!(
            "$p = Start-Process -FilePath {} -ArgumentList @('agent', '--allowed-exe', {}) -Verb RunAs -WindowStyle Hidden -PassThru\r\n",
            quote_ps(&self_exe.to_string_lossy()),
            quote_ps(&allowed_exe.to_string_lossy()),
        );
        // 写入 UTF-8 with BOM 防止 Windows PowerShell 5.1 中文路径乱码
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(script.as_bytes());
        fs::write(&launcher_ps1, bytes)?;

        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &launcher_ps1.to_string_lossy(),
            ])
            .status()?;

        if !status.success() {
            anyhow::bail!("管理员权限授予被取消或失败");
        }

        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if let Some(resp) = Self::try_request_agent(&json!({ "type": "ping" })) {
                if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                    return Ok(());
                }
            }
        }
        anyhow::bail!("等待常驻提权代理就绪超时")
    }

    fn launch_via_elevated_agent(exe: &Path, args: &[String], cwd: &Path) -> anyhow::Result<u32> {
        Self::ensure_elevated_agent_running(exe)?;
        let resp = Self::try_request_agent(&json!({
            "type": "launch",
            "exe": exe.to_string_lossy(),
            "args": args,
            "cwd": cwd.to_string_lossy(),
        }))
        .ok_or_else(|| anyhow::anyhow!("提权代理未响应"))?;

        if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            let pid = resp.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            Ok(pid)
        } else {
            let err = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("未知提权启动错误");
            anyhow::bail!("{}", err)
        }
    }

    fn launch_via_powershell_runas(
        exe: &Path,
        args: &[String],
        cwd: &Path,
    ) -> anyhow::Result<u32> {
        let quote_ps = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let ps_args = args
            .iter()
            .map(|a| quote_ps(a))
            .collect::<Vec<_>>()
            .join(", ");
        let pid_file = std::env::temp_dir().join("y3-helper-last-pid.txt");
        let _ = fs::remove_file(&pid_file);

        let script = format!(
            "$p = Start-Process -FilePath {} -ArgumentList @({}) -WorkingDirectory {} -Verb RunAs -PassThru\r\nif ($p) {{ Set-Content -Path {} -Value $p.Id -Encoding ASCII }}\r\n",
            quote_ps(&exe.to_string_lossy()),
            ps_args,
            quote_ps(&cwd.to_string_lossy()),
            quote_ps(&pid_file.to_string_lossy()),
        );
        let ps1_path = std::env::temp_dir().join("y3-helper-runas-direct.ps1");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(script.as_bytes());
        fs::write(&ps1_path, bytes)?;

        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &ps1_path.to_string_lossy(),
            ])
            .status()?;

        if !status.success() {
            anyhow::bail!("通过 UAC 提权启动失败或被用户取消");
        }
        let pid = fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        Ok(pid)
    }

    /// 终止游戏进程：
    /// - 若传入 `project_path` 且 `known_pids` 非空，优先终止 `known_pids` 以及命令行包含该项目路径的 `Game_x64h.exe`（多项目安全隔离！）
    /// - 若 `kill_all = true`，终止所有游戏模式的 `Game_x64h.exe`（自动排除 `type@editor,` 的编辑器窗口）
    pub fn kill_game(
        project_path: Option<&Path>,
        known_pids: &[u32],
        kill_all: bool,
    ) -> anyhow::Result<Vec<u32>> {
        let mut killed_pids = Vec::new();

        for &pid in known_pids {
            if Self::taskkill_pid(pid) {
                killed_pids.push(pid);
            }
        }

        // 查询运行中的 Game_x64h.exe 进程命令行以精准匹配目标项目
        if kill_all || project_path.is_some() {
            let filter_str = project_path.map(|p| p.to_string_lossy().to_lowercase());
            let ps_cmd = "Get-CimInstance Win32_Process -Filter \"Name = 'Game_x64h.exe'\" | Select-Object ProcessId, CommandLine | ConvertTo-Json -Compress";
            if let Ok(output) = Command::new("powershell")
                .args(["-NoProfile", "-Command", ps_cmd])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Ok(val) = serde_json::from_str::<Value>(stdout.trim()) {
                    let items = match val {
                        Value::Array(arr) => arr,
                        obj @ Value::Object(_) => vec![obj],
                        _ => Vec::new(),
                    };
                    for item in items {
                        let pid = match item.get("ProcessId").and_then(|v| v.as_u64()) {
                            Some(p) => p as u32,
                            None => continue,
                        };
                        if killed_pids.contains(&pid) {
                            continue;
                        }
                        let cmdline = item
                            .get("CommandLine")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_lowercase();

                        // 绝不误杀编辑器主窗口 (type@editor,subtype@author)
                        if cmdline.contains("type@editor,") {
                            continue;
                        }

                        let should_kill = if kill_all {
                            true
                        } else if let Some(ref proj_lower) = filter_str {
                            let proj_slash = proj_lower.replace('\\', "/");
                            cmdline.contains(proj_lower) || cmdline.contains(&proj_slash)
                        } else {
                            false
                        };

                        if should_kill && Self::taskkill_pid(pid) {
                            killed_pids.push(pid);
                        }
                    }
                }
            }
        }

        Ok(killed_pids)
    }

    pub fn taskkill_pid(pid: u32) -> bool {
        if let Ok(out) = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output()
        {
            if out.status.success() {
                return true;
            }
        }
        // 若当前非管理员权限无法终止管理员权限的 Game_x64h.exe，交由常驻提权代理终止
        if let Some(resp) = Self::try_request_agent(&json!({
            "type": "kill_pid",
            "pid": pid
        })) {
            if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                return true;
            }
        }
        false
    }

    /// 计算与原版 `src/launchAgent/protocol.ts` 一致的提权代理命名管道名称
    pub fn get_agent_pipe_name() -> String {
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "unknown".to_string());
        let key = format!("{}|{}", user, std::env::consts::ARCH);
        let mut hasher = Sha1::new();
        hasher.update(key.as_bytes());
        let h = hex::encode(hasher.finalize());
        format!(r"\\.\pipe\y3-helper-agent-{}", &h[..12])
    }

    /// 内置常驻免 UAC 提权代理守护服务（只需授权一次，跨会话常驻复用）
    pub fn run_elevated_agent_stdio_loop(allowed_exe: &Path) -> anyhow::Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let token = uuid::Uuid::new_v4().to_string();

        let state_json = json!({
            "pid": std::process::id(),
            "port": port,
            "token": token,
            "allowed_exe": allowed_exe.to_string_lossy(),
        });
        fs::write(
            Self::agent_state_file(),
            serde_json::to_string_pretty(&state_json)?,
        )?;

        let allowed_norm = allowed_exe.to_string_lossy().to_lowercase();

        for stream_res in listener.incoming() {
            let mut stream = match stream_res {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut line = String::new();
            {
                let mut reader = BufReader::new(&stream);
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
            }
            let msg: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let req_token = msg.get("token").and_then(|v| v.as_str()).unwrap_or("");
            if req_token != token {
                let _ = writeln!(stream, "{}", json!({ "ok": false, "error": "Invalid token" }));
                continue;
            }

            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match msg_type {
                "ping" => {
                    let _ = writeln!(stream, "{}", json!({ "ok": true, "pid": std::process::id() }));
                }
                "shutdown" => {
                    let _ = writeln!(stream, "{}", json!({ "ok": true }));
                    break;
                }
                "launch" => {
                    let exe = msg.get("exe").and_then(|v| v.as_str()).unwrap_or("");
                    let exe_lower = exe.to_lowercase();
                    if !exe_lower.ends_with("game_x64h.exe")
                        && !exe_lower.ends_with("editor.exe")
                        && exe_lower != allowed_norm
                    {
                        let _ = writeln!(
                            stream,
                            "{}",
                            json!({ "ok": false, "error": "Executable not in whitelist" })
                        );
                        continue;
                    }
                    let args: Vec<String> = msg
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let cwd = msg.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
                    match Command::new(exe)
                        .args(&args)
                        .current_dir(cwd)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                    {
                        Ok(child) => {
                            let _ = writeln!(
                                stream,
                                "{}",
                                json!({ "ok": true, "pid": child.id() })
                            );
                        }
                        Err(e) => {
                            let _ = writeln!(
                                stream,
                                "{}",
                                json!({ "ok": false, "error": e.to_string() })
                            );
                        }
                    }
                }
                "kill_pid" => {
                    let pid = msg.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let ok = if pid > 0 {
                        Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/T", "/F"])
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    let _ = writeln!(stream, "{}", json!({ "ok": ok, "pid": pid }));
                }
                _ => {
                    let _ = writeln!(stream, "{}", json!({ "ok": false, "error": "Unknown command" }));
                }
            }
            let _ = stream.flush();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multi_player_roles_and_archives() {
        let tmp = tempfile::tempdir().unwrap();
        let proj_dir = tmp.path().join("Proj");
        let map_dir = proj_dir.join("maps/EntryMap");
        fs::create_dir_all(map_dir.join("script")).unwrap();
        fs::write(
            proj_dir.join("header.project"),
            r#"{"entry_map": {"id": 1}}"#,
        )
        .unwrap();
        fs::write(map_dir.join("header.map"), r#"{"id": 1}"#).unwrap();
        fs::write(
            map_dir.join("campinfo.json"),
            r#"{"roles": [{"aid": 1}, {"aid": 3}, {"aid": 31}, {"aid": 32}]}"#,
        )
        .unwrap();

        let ctx = ProjectContext::load_from_path(&proj_dir).unwrap();
        let roles = GameLauncher::load_available_multi_player_ids(&ctx, None);
        assert_eq!(roles, vec![1, 3]);

        let nicks =
            GameLauncher::prepare_multi_player_nicknames(&ctx, &[1, 3], Some("开发者A")).unwrap();
        assert_eq!(nicks[&1], "开发者A");
        assert_eq!(nicks[&3], "Player 3");
        assert!(map_dir.join("archive/archive_storage.json").is_file());
    }
}
