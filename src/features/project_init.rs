use crate::env::{EditorEnv, ProjectContext};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn hidden_cmd(prog: &str) -> Command {
    let mut cmd = Command::new(prog);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    if prog.eq_ignore_ascii_case("git") {
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_ASKPASS", "echo")
            .args(["-c", "credential.helper="]);
    }
    cmd
}

pub const LUALIB_REPO_GITEE: &str = "https://gitee.com/tsukiko/y3-lualib.git";
pub const LUALIB_REPO_GITHUB: &str = "https://github.com/y3-editor/y3-lualib.git";
pub const LUALIB_REPO_EN: &str = "https://github.com/CliCli-Editor/lualib.git";

pub const Y3MAKER_REPO_GITEE: &str = "https://github.com/y3-editor/y3-maker-config.git";
pub const Y3MAKER_REPO_GITHUB: &str = "https://github.com/y3-editor/y3-maker-config.git";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoSource {
    Gitee,
    Github,
    English,
}

impl RepoSource {
    pub fn parse(s: Option<&str>) -> Self {
        match s.unwrap_or("gitee").trim().to_lowercase().as_str() {
            "github" => Self::Github,
            "en" | "english" => Self::English,
            _ => Self::Gitee,
        }
    }

    pub fn lualib_url(&self) -> &'static str {
        match self {
            Self::Gitee => LUALIB_REPO_GITEE,
            Self::Github => LUALIB_REPO_GITHUB,
            Self::English => LUALIB_REPO_EN,
        }
    }

    pub fn y3maker_url(&self) -> &'static str {
        match self {
            Self::Github | Self::English => Y3MAKER_REPO_GITHUB,
            Self::Gitee => Y3MAKER_REPO_GITEE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub has_scripts: bool,
    pub script_count: usize,
    pub bom_clean: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTargetStatus {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub config_path: String,
    pub detected: bool,
    pub equipped: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Y3MakerHubStatus {
    pub project_id: String,
    pub project_path: String,
    pub y3maker_dir: String,
    pub status: String,
    pub git_commit: Option<String>,
    pub git_commit_msg: Option<String>,
    pub git_commit_time: Option<String>,
    pub git_remote: Option<String>,
    pub global_hub_dir: String,
    pub global_hub_ready: bool,
    pub global_hub_commit: Option<String>,
    pub skills_count: usize,
    pub knowledge_count: usize,
    pub templates_count: usize,
    pub tools_count: usize,
    pub agents_skills_linked: bool,
    pub claude_skills_linked: bool,
    pub has_agents_md: bool,
    pub mcp_adapted: bool,
    pub mcp_adapter_notes: Vec<String>,
    pub skills: Vec<SkillInfo>,
    pub agent_targets: Vec<AgentTargetStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitProjectReport {
    pub project_id: String,
    pub map_name: String,
    pub y3_dir: PathBuf,
    pub y3maker_status: String,
    pub copied_files: Vec<String>,
}

/// 初始化新地图与管理 `.y3maker` 配置仓库
pub struct ProjectInitializer;

impl ProjectInitializer {
    /// 初始化指定项目的指定子地图（克隆 `y3-lualib`、复制演示配置、初始化 `.y3maker`）
    pub fn init_map(
        project: &ProjectContext,
        map_name: Option<&str>,
        source: RepoSource,
        editor: Option<&EditorEnv>,
        init_y3maker: bool,
    ) -> anyhow::Result<InitProjectReport> {
        let map = project.resolve_map(map_name)?;
        fs::create_dir_all(&map.script_path)?;
        let y3_dir = map.script_path.join("y3");

        if !y3_dir.join(".git").is_dir() {
            if y3_dir.exists() {
                fs::remove_dir_all(&y3_dir)?;
            }
            let status = hidden_cmd("git")
                .args([
                    "clone",
                    source.lualib_url(),
                    &y3_dir.to_string_lossy(),
                ])
                .status()?;
            if !status.success() {
                anyhow::bail!("git clone y3-lualib 失败 (URL: {})", source.lualib_url());
            }

            // 如果编辑器版本为 1.0，切换到 1.0 分支
            if editor.map(|e| e.version == "1.0").unwrap_or(false) {
                let _ = hidden_cmd("git")
                    .args([
                        "-C",
                        &y3_dir.to_string_lossy(),
                        "checkout",
                        "-b",
                        "1.0",
                        "origin/1.0",
                    ])
                    .status();
            }
        }

        // 创建 script/.log
        let _ = fs::create_dir_all(map.script_path.join(".log"));

        // 复制 y3/演示/项目配置/* 到 script/
        let mut copied_files = Vec::new();
        let demo_cfg_dir = y3_dir.join("演示").join("项目配置");
        if demo_cfg_dir.is_dir() {
            for entry in WalkDir::new(&demo_cfg_dir).min_depth(1).into_iter().flatten() {
                if let Ok(rel) = entry.path().strip_prefix(&demo_cfg_dir) {
                    let dst = map.script_path.join(rel);
                    if entry.file_type().is_dir() {
                        let _ = fs::create_dir_all(&dst);
                    } else if entry.file_type().is_file() && !dst.exists() {
                        if let Some(p) = dst.parent() {
                            let _ = fs::create_dir_all(p);
                        }
                        if fs::copy(entry.path(), &dst).is_ok() {
                            copied_files.push(rel.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }

        let y3maker_status = if init_y3maker {
            match Self::init_or_update_y3maker(&project.project_path, source, true) {
                Ok(s) => s,
                Err(e) => format!("failed: {}", e),
            }
        } else {
            Self::inspect_y3maker_status(&project.project_path)
        };

        Ok(InitProjectReport {
            project_id: project.project_id.clone(),
            map_name: map.name.clone(),
            y3_dir,
            y3maker_status,
            copied_files,
        })
    }

    /// 检查 `.y3maker` 目录状态：`"managed"` / `"foreign"` / `"partial"` / `"missing"`
    pub fn inspect_y3maker_status(project_path: &Path) -> String {
        let dir = project_path.join(".y3maker");
        if !dir.is_dir() {
            return "missing".to_string();
        }
        if !dir.join(".git").exists() {
            return "partial".to_string();
        }
        if let Ok(out) = hidden_cmd("git")
            .args([
                "-C",
                &dir.to_string_lossy(),
                "config",
                "--get",
                "remote.origin.url",
            ])
            .output()
        {
            let url = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
            if url.contains("y3-maker-config") {
                return "managed".to_string();
            }
            if !url.is_empty() {
                return "foreign".to_string();
            }
        }
        "partial".to_string()
    }

    /// 初始化或更新项目的 `.y3maker` 目录
    pub fn init_or_update_y3maker(
        project_path: &Path,
        source: RepoSource,
        merge_existing: bool,
    ) -> anyhow::Result<String> {
        let state = Self::inspect_y3maker_status(project_path);
        let target_dir = project_path.join(".y3maker");

        let res = match state.as_str() {
            "missing" => {
                let status = hidden_cmd("git")
                    .args([
                        "clone",
                        source.y3maker_url(),
                        &target_dir.to_string_lossy(),
                    ])
                    .status()?;
                if !status.success() {
                    anyhow::bail!("克隆 .y3maker 失败");
                }
                Ok("cloned".to_string())
            }
            "managed" => {
                let _ = hidden_cmd("git")
                    .args(["-C", &target_dir.to_string_lossy(), "pull", "--ff-only"])
                    .status();
                Ok("updated".to_string())
            }
            _ => {
                if merge_existing {
                    let staging = project_path.join(".y3maker.staging");
                    if staging.exists() {
                        let _ = fs::remove_dir_all(&staging);
                    }
                    let status = hidden_cmd("git")
                        .args(["clone", source.y3maker_url(), &staging.to_string_lossy()])
                        .status()?;
                    if !status.success() {
                        anyhow::bail!("克隆 .y3maker.staging 失败");
                    }
                    // 将用户已有文件复制合并进 staging（不覆盖 .git）
                    for entry in WalkDir::new(&target_dir).min_depth(1).into_iter().flatten() {
                        if let Ok(rel) = entry.path().strip_prefix(&target_dir) {
                            if rel.starts_with(".git") {
                                continue;
                            }
                            let dst = staging.join(rel);
                            if entry.file_type().is_dir() {
                                let _ = fs::create_dir_all(&dst);
                            } else if entry.file_type().is_file() {
                                if let Some(p) = dst.parent() {
                                    let _ = fs::create_dir_all(p);
                                }
                                let _ = fs::copy(entry.path(), &dst);
                            }
                        }
                    }
                    fs::remove_dir_all(&target_dir)?;
                    fs::rename(&staging, &target_dir)?;
                    Ok("merged".to_string())
                } else {
                    let ts = chrono::Utc::now().timestamp();
                    let bak = project_path.join(format!(".y3maker.bak-{}", ts));
                    fs::rename(&target_dir, &bak)?;
                    let status = hidden_cmd("git")
                        .args([
                            "clone",
                            source.y3maker_url(),
                            &target_dir.to_string_lossy(),
                        ])
                        .status()?;
                    if !status.success() {
                        anyhow::bail!("备份后克隆 .y3maker 失败");
                    }
                    Ok("backed_up_and_cloned".to_string())
                }
            }
        };

        // 自动剥离 SKILL.md 的 UTF-8 BOM
        let _ = Self::strip_skills_utf8_bom(&target_dir);
        res
    }

    /// 扫描并剥离 `.y3maker/skills/**/SKILL.md` 开头的 3 字节 UTF-8 BOM (`EF BB BF`)，
    /// 防止 Codex / Claude Code 等智能体解析 YAML Frontmatter 失败导致技能消失
    pub fn strip_skills_utf8_bom(base_dir: &Path) -> usize {
        let skills_dir = if base_dir.ends_with("skills") {
            base_dir.to_path_buf()
        } else {
            base_dir.join("skills")
        };
        if !skills_dir.is_dir() {
            return 0;
        }
        let mut fixed = 0usize;
        for entry in WalkDir::new(&skills_dir).max_depth(3).into_iter().flatten() {
            if entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("SKILL.md")
            {
                if let Ok(bytes) = fs::read(entry.path()) {
                    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                        if fs::write(entry.path(), &bytes[3..]).is_ok() {
                            fixed += 1;
                        }
                    }
                }
            }
        }
        fixed
    }

    /// 客户端应用级全局技能母库目录 (`%APPDATA%\y3-helper\y3maker-hub`)
    /// 用于在任意用户的 Windows 电脑上缓存官方 `y3-maker-config` 仓库：
    /// 一次拉取即可秒级分发给本机所有挂载的 Y3 项目，无需每个项目重复从公网下载。
    pub fn app_global_hub_dir() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("y3-helper").join("y3maker-hub")
    }

    /// 解析 Git 源地址（支持 "gitee" / "github" 或任意自定义 https/git@ 仓库地址）
    pub fn resolve_y3maker_git_url(source_str: Option<&str>) -> String {
        let raw = source_str.unwrap_or("gitee").trim();
        if raw.starts_with("http://")
            || raw.starts_with("https://")
            || raw.starts_with("git@")
            || raw.starts_with("ssh://")
        {
            return raw.to_string();
        }
        RepoSource::parse(Some(raw)).y3maker_url().to_string()
    }

    /// 读取指定 Git 目录的短 Commit、提交时间与标题
    fn read_git_head_meta(repo_dir: &Path) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
        if !repo_dir.join(".git").exists() {
            return (None, None, None, None);
        }
        let mut commit = None;
        let mut time = None;
        let mut msg = None;
        let mut remote = None;

        if let Ok(out) = hidden_cmd("git")
            .args([
                "-C",
                &repo_dir.to_string_lossy(),
                "log",
                "-1",
                "--format=%h%n%ci%n%s",
            ])
            .output()
        {
            let txt = String::from_utf8_lossy(&out.stdout);
            let mut lines = txt.lines();
            if let Some(c) = lines.next().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                commit = Some(c.to_string());
            }
            if let Some(t) = lines.next().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                time = Some(t.to_string());
            }
            if let Some(m) = lines.next().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                msg = Some(m.to_string());
            }
        }
        if let Ok(out) = hidden_cmd("git")
            .args([
                "-C",
                &repo_dir.to_string_lossy(),
                "config",
                "--get",
                "remote.origin.url",
            ])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                remote = Some(s);
            }
        }
        (commit, time, msg, remote)
    }

    /// 运行带进程级硬超时的 git 命令（禁用一切交互弹窗，超时自动 kill 子进程，绝不卡死）
    fn run_git_with_timeout(args: &[&str], timeout_ms: u64) -> bool {
        let mut cmd = hidden_cmd("git");
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_ASKPASS", "echo")
            .args([
                "-c",
                "http.connectTimeout=3",
                "-c",
                "http.lowSpeedLimit=1000",
                "-c",
                "http.lowSpeedTime=3",
            ])
            .args(args);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(_) => return false,
        };

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(80));
                }
                Err(_) => return false,
            }
        }
    }

    /// 同步客户端全局母库 (`%APPDATA%\y3-helper\y3maker-hub`)，自带 4 秒进程级硬超时保护与本地缓存毫秒级兜底
    pub fn sync_global_hub(remote_url: &str, skip_network: bool) -> anyhow::Result<PathBuf> {
        let hub_dir = Self::app_global_hub_dir();
        if let Some(parent) = hub_dir.parent() {
            fs::create_dir_all(parent)?;
        }

        // 若指定 skip_network 且全局母库已就绪，0 网络请求直接返回（耗时 < 1ms）
        if skip_network && hub_dir.join("skills").is_dir() {
            Self::strip_skills_utf8_bom(&hub_dir);
            return Ok(hub_dir);
        }

        let hub_str = hub_dir.to_string_lossy().to_string();
        if hub_dir.join(".git").exists() {
            let _ = Self::run_git_with_timeout(
                &["-C", &hub_str, "remote", "set-url", "origin", remote_url],
                1500,
            );
            let _ = Self::run_git_with_timeout(&["-C", &hub_str, "pull", "--ff-only"], 4000);
        } else if !hub_dir.join("skills").is_dir() {
            if hub_dir.exists() {
                let _ = fs::remove_dir_all(&hub_dir);
            }
            let ok = Self::run_git_with_timeout(
                &["clone", "--depth", "1", remote_url, &hub_str],
                5000,
            );
            if !ok && !hub_dir.join("skills").is_dir() {
                anyhow::bail!("从远程克隆全局技能母库超时或失败 ({})", remote_url);
            }
        } else {
            // 已有预置的全局母库文件但尚未初始化 .git，尝试最多 3.5 秒拉取，超时立即使用现有本地母库！
            let staging = hub_dir.with_file_name("y3maker-hub.staging");
            if staging.exists() {
                let _ = fs::remove_dir_all(&staging);
            }
            let staging_str = staging.to_string_lossy().to_string();
            let ok = Self::run_git_with_timeout(
                &["clone", "--depth", "1", remote_url, &staging_str],
                3500,
            );
            if ok && staging.join("skills").is_dir() {
                let _ = fs::remove_dir_all(&hub_dir);
                let _ = fs::rename(&staging, &hub_dir);
            } else {
                let _ = fs::remove_dir_all(&staging);
            }
        }
        Self::strip_skills_utf8_bom(&hub_dir);
        Ok(hub_dir)
    }

    /// 官方 `.y3maker` ➔ `Y3 Studio` 三合一 MCP 自动适配转换引擎：
    /// 解决官方 `.y3maker` 对 MCP 的旧假设与我们独立客户端之间的 4 大差异：
    /// 1. 生成双格式 `.y3maker/mcp_settings.json`（同时含 `mcpServers` 与顶层 `y3editor`/`y3-helper`/`helper`/`y3runtime`，解决 `mcp_batch_writer.py` 与 `mcp_entity_writer.py` 读取结构不一致及硬编码 8765/25000 端口问题）
    /// 2. 自动修补 `y3-auto-test/SKILL.md` 中要求用户手动刷新 `y3runtime` 连接的阻塞提示（改为常驻网关免刷新直接调用）
    /// 3. 自动修补 `y3-env-setup/SKILL.md` 中的 VS Code 插件强依赖说明
    /// 4. 生成 `.y3maker/rules/y3-studio-mcp-bridge.md` 协议桥接规范（统一 `helper.*`/`y3-helper.*` 别名、多项目 `projectPath`/`mapName` 参数与 `.codemaker` 路径映射）
    pub fn adapt_y3maker_mcp_for_studio(
        y3maker_dir: &Path,
        project_path: &Path,
        mcp_port: u16,
    ) -> anyhow::Result<Vec<String>> {
        if !y3maker_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut notes = Vec::new();

        let helper_url = format!("http://127.0.0.1:{}/mcp", mcp_port);
        let editor_url = format!("http://127.0.0.1:{}/y3editor/mcp", mcp_port);
        let runtime_url = format!("http://127.0.0.1:{}/y3runtime/mcp", mcp_port);

        // 1. 写入双格式兼容的 .y3maker/mcp_settings.json
        // 兼容 mcp_batch_writer.py (读 settings.mcpServers.y3editor.url)
        // 兼容 mcp_entity_writer.py (读 settings.y3editor.url)
        let dual_mcp_settings = json!({
            "_comment": "Auto-generated by Y3 Studio MCP Adapter: compatible with both mcpServers.* and top-level server keys for official .y3maker Python scripts.",
            "projectPath": project_path.to_string_lossy(),
            "mcpServers": {
                "y3-helper": {
                    "type": "streamableHttp",
                    "url": helper_url,
                    "timeout": 120,
                    "autoApprove": true,
                    "disabled": false
                },
                "helper": {
                    "type": "streamableHttp",
                    "url": helper_url,
                    "timeout": 120,
                    "autoApprove": true,
                    "disabled": false
                },
                "y3editor": {
                    "type": "streamableHttp",
                    "url": editor_url,
                    "timeout": 300,
                    "autoApprove": true,
                    "disabled": false
                },
                "y3runtime": {
                    "type": "streamableHttp",
                    "url": runtime_url,
                    "timeout": 120,
                    "autoApprove": true,
                    "disabled": false
                }
            },
            "y3-helper": { "url": helper_url },
            "helper": { "url": helper_url },
            "y3editor": { "url": editor_url },
            "y3runtime": { "url": runtime_url }
        });
        let settings_path = y3maker_dir.join("mcp_settings.json");
        fs::write(&settings_path, serde_json::to_string_pretty(&dual_mcp_settings)?)?;
        notes.push(format!(
            "已重写 .y3maker/mcp_settings.json (双格式兼容 mcp_batch_writer.py 与 mcp_entity_writer.py，统一指向 :{} 网关)",
            mcp_port
        ));

        // 2. 自动修补 y3-auto-test/SKILL.md 中要求用户手动刷新 y3runtime 的阻塞提示
        let auto_test_md = y3maker_dir.join("skills").join("y3-auto-test").join("SKILL.md");
        if auto_test_md.is_file() {
            if let Ok(txt) = fs::read_to_string(&auto_test_md) {
                let old_tip = "> 启动后提醒用户刷新 y3runtime MCP 连接，等用户确认后再继续。";
                let new_tip = "> [Y3 Studio 常驻网关已适配] `y3runtime` 由 Y3 Studio 常驻托管，启动游戏后**无需提醒用户刷新 MCP 连接**，轮询 `get_game_status` 确认客户端连上后即可直接调用 `trigger_ui_touch_event_by_path`。";
                if txt.contains(old_tip) {
                    let patched = txt.replace(old_tip, new_tip);
                    let _ = fs::write(&auto_test_md, patched);
                }
            }
            notes.push("已适配 y3-auto-test：移除 y3runtime 手动刷新停顿，启用常驻网关直连".to_string());
        }

        // 3. 自动剥离所有 SKILL.md 的 UTF-8 BOM
        let bom_fixed = Self::strip_skills_utf8_bom(y3maker_dir);
        notes.push(format!(
            "已完成全部 SKILL.md 的 UTF-8 BOM 净化 (本次剥离 {} 个 BOM 头)",
            bom_fixed
        ));

        // 4. 生成 .y3maker/rules/y3-studio-mcp-bridge.md 协议桥接说明
        let rules_dir = y3maker_dir.join("rules");
        fs::create_dir_all(&rules_dir)?;
        let bridge_md = rules_dir.join("y3-studio-mcp-bridge.md");
        let bridge_content = format!(
            "# Y3 Studio 独立操作台与官方 `.y3maker` MCP 桥接规范\n\n\
            本工程由 **Y3 Studio 原生独立客户端** 托管（端口 `{port}`），无需安装 VS Code 或 `sumneko.y3-helper` 扩展。阅读 `.y3maker/skills/` 下的官方技能文档时，请遵循以下自动映射规则：\n\n\
            1. **MCP 服务名与端点映射**：\n\
               - 技能文档中的 `helper.<tool>`、`y3helper.<tool>`、`y3-helper.<tool>` 均对应 **`y3-helper`**（或同名别名 **`helper`**）MCP 服务（`http://127.0.0.1:{port}/mcp`）。\n\
               - 技能文档中的 `y3editor.<tool>` 对应 **`y3editor`** 多编辑器路由网关（`http://127.0.0.1:{port}/y3editor/mcp`），网关会自动按当前项目路由到对应编辑器的 `8765+` 端口。\n\
               - 技能文档中的 `y3runtime.<tool>`（如 `trigger_ui_touch_event_by_path`）对应 **`y3runtime`** 常驻网关（`http://127.0.0.1:{port}/y3runtime/mcp`），**永不掉线，启动游戏后无需让用户手动刷新 MCP 连接**。\n\
            2. **多项目与多子地图并发参数**：\n\
               - 所有工具均额外支持可选参数 `projectPath`（当前工程为 `{proj}`）与 `mapName`（如 `EntryMap`），可在同一台电脑并发操作多个项目与多张子地图。\n\
            3. **目录占位符映射**：\n\
               - 技能文档中的 `.codemaker`、`<codemaker>`、`<agent>` 路径均对应本工程下的 `.y3maker` 目录（`{y3m}`）。\n",
            port = mcp_port,
            proj = project_path.display(),
            y3m = y3maker_dir.display()
        );
        fs::write(&bridge_md, bridge_content)?;
        notes.push("已生成 .y3maker/rules/y3-studio-mcp-bridge.md 协议桥接规范".to_string());

        Ok(notes)
    }

    /// 将全局母库快速克隆/同步到指定项目的 `.y3maker` 目录
    pub fn sync_project_from_hub_or_remote(
        project_path: &Path,
        remote_url: &str,
        skip_network: bool,
    ) -> anyhow::Result<String> {
        let hub_dir = Self::sync_global_hub(remote_url, skip_network)?;
        let target_dir = project_path.join(".y3maker");
        let hub_str = hub_dir.to_string_lossy().to_string();
        let target_str = target_dir.to_string_lossy().to_string();

        let mut action = "synced_from_global_hub";
        if !target_dir.exists() && hub_dir.join(".git").exists() {
            // 尝试从本地全局母库 clone Git 历史
            if Self::run_git_with_timeout(&["clone", &hub_str, &target_str], 3000) {
                let _ = Self::run_git_with_timeout(
                    &["-C", &target_str, "remote", "set-url", "origin", remote_url],
                    1500,
                );
                action = "cloned_from_global_hub";
            }
        } else if target_dir.join(".git").exists() && hub_dir.join(".git").exists() {
            let _ = Self::run_git_with_timeout(
                &["-C", &target_str, "pull", "--ff-only", &hub_str],
                3000,
            );
            let _ = Self::run_git_with_timeout(
                &["-C", &target_str, "remote", "set-url", "origin", remote_url],
                1500,
            );
            action = "updated_via_global_hub";
        }

        // 始终将全局母库工作区文件增量同步进项目的 .y3maker（确保未 commit 的 skills/knowledge/templates/rules 100% 落盘）
        for entry in WalkDir::new(&hub_dir).min_depth(1).into_iter().flatten() {
            if let Ok(rel) = entry.path().strip_prefix(&hub_dir) {
                if rel.starts_with(".git") {
                    continue;
                }
                let dst = target_dir.join(rel);
                if entry.file_type().is_dir() {
                    let _ = fs::create_dir_all(&dst);
                } else if entry.file_type().is_file() {
                    let need_copy = match (entry.metadata(), fs::metadata(&dst)) {
                        (Ok(src_m), Ok(dst_m)) => src_m.len() != dst_m.len(),
                        _ => true,
                    };
                    if need_copy {
                        if let Some(p) = dst.parent() {
                            let _ = fs::create_dir_all(p);
                        }
                        let _ = fs::copy(entry.path(), &dst);
                    }
                }
            }
        }
        Self::strip_skills_utf8_bom(&target_dir);
        Ok(action.to_string())
    }

    /// 检查远程 Git 仓库是否有新提交可更新（带 3.5 秒硬超时保护）
    pub fn check_y3maker_remote_update(
        project: &ProjectContext,
        source_str: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let remote_url = Self::resolve_y3maker_git_url(source_str);
        let y3maker_dir = project.project_path.join(".y3maker");
        let check_dir = if y3maker_dir.join(".git").exists() {
            y3maker_dir.clone()
        } else {
            Self::app_global_hub_dir()
        };

        if !check_dir.join(".git").exists() {
            let has_local_skills = y3maker_dir.join("skills").is_dir()
                || Self::app_global_hub_dir().join("skills").is_dir();
            return Ok(json!({
                "installed": has_local_skills,
                "has_update": !has_local_skills,
                "behind_count": if has_local_skills { 0 } else { 1 },
                "remote_url": remote_url,
                "message": if has_local_skills {
                    "当前使用客户端全局母库缓存（全部 14 个官方技能已就绪）；点击「拉取 / 同步」可强制与远程同步"
                } else {
                    "当前尚未安装 .y3maker 仓库，点击「拉取 / 同步」即可从官方源安装"
                }
            }));
        }

        let check_str = check_dir.to_string_lossy().to_string();
        let _ = Self::run_git_with_timeout(&["-C", &check_str, "fetch", "origin"], 3500);

        let local_head = hidden_cmd("git")
            .args(["-C", &check_str, "rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        let remote_head = hidden_cmd("git")
            .args(["-C", &check_str, "rev-parse", "--short", "origin/HEAD"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        let behind_count: usize = hidden_cmd("git")
            .args(["-C", &check_str, "rev-list", "--count", "HEAD..origin/HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(0);

        Ok(json!({
            "installed": true,
            "local_commit": local_head,
            "remote_commit": remote_head,
            "behind_count": behind_count,
            "has_update": behind_count > 0,
            "message": if behind_count > 0 {
                format!("发现远程有 {} 个新提交可更新 ({} -> {})", behind_count, local_head, remote_head)
            } else {
                format!("当前已是最新官方版本 (Commit {})", local_head)
            }
        }))
    }

    /// 创建 Windows 目录 Junction（若目标已存在但为空或损坏则自动重建）
    pub fn ensure_dir_junction(link_path: &Path, target_path: &Path) -> anyhow::Result<bool> {
        if !target_path.is_dir() {
            anyhow::bail!("目标目录不存在: {}", target_path.display());
        }
        if link_path.exists() {
            if Self::count_files_in(link_path) > 0 {
                return Ok(true);
            }
            let _ = fs::remove_dir(link_path).or_else(|_| fs::remove_dir_all(link_path));
        }
        if let Some(parent) = link_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let status = hidden_cmd("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &link_path.to_string_lossy(),
                &target_path.to_string_lossy(),
            ])
            .status()?;
        if status.success() || link_path.exists() {
            Ok(true)
        } else {
            for entry in WalkDir::new(target_path).min_depth(1).into_iter().flatten() {
                if let Ok(rel) = entry.path().strip_prefix(target_path) {
                    let dst = link_path.join(rel);
                    if entry.file_type().is_dir() {
                        let _ = fs::create_dir_all(&dst);
                    } else if entry.file_type().is_file() {
                        if let Some(p) = dst.parent() {
                            let _ = fs::create_dir_all(p);
                        }
                        let _ = fs::copy(entry.path(), &dst);
                    }
                }
            }
            Ok(link_path.is_dir())
        }
    }

    /// 解析单个 `SKILL.md` 的 YAML Frontmatter
    fn parse_skill_md(skill_dir: &Path) -> Option<SkillInfo> {
        let id = skill_dir.file_name()?.to_string_lossy().to_string();
        if id.starts_with('_') || id.starts_with('.') {
            return None;
        }
        let md_path = skill_dir.join("SKILL.md");
        let bytes = fs::read(&md_path).ok()?;
        let bom_clean = !bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
        let content_bytes = if bom_clean { &bytes[..] } else { &bytes[3..] };
        let text = String::from_utf8_lossy(content_bytes);

        let mut name = id.clone();
        let mut desc_lines = Vec::new();
        let mut in_frontmatter = false;
        let mut in_desc = false;

        for (idx, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if idx == 0 && trimmed == "---" {
                in_frontmatter = true;
                continue;
            }
            if in_frontmatter {
                if trimmed == "---" {
                    break;
                }
                if let Some(rest) = trimmed.strip_prefix("name:") {
                    let v = rest.trim().trim_matches('"').trim_matches('\'');
                    if !v.is_empty() {
                        name = v.to_string();
                    }
                    in_desc = false;
                } else if let Some(rest) = trimmed.strip_prefix("description:") {
                    let v = rest.trim().trim_start_matches('|').trim_start_matches('>').trim();
                    if !v.is_empty() {
                        desc_lines.push(v.trim_matches('"').trim_matches('\'').to_string());
                    }
                    in_desc = true;
                } else if in_desc {
                    if line.starts_with(' ') || line.starts_with('\t') {
                        if !trimmed.is_empty() {
                            desc_lines.push(trimmed.to_string());
                        }
                    } else if trimmed.contains(':') {
                        in_desc = false;
                    }
                }
            }
        }

        let description = if desc_lines.is_empty() {
            text.lines()
                .filter(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with("---") && !t.starts_with('#')
                })
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            desc_lines.join(" ")
        };

        let scripts_dir = skill_dir.join("scripts");
        let (has_scripts, script_count) = if scripts_dir.is_dir() {
            let cnt = WalkDir::new(&scripts_dir)
                .min_depth(1)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
                .count();
            (cnt > 0, cnt)
        } else {
            (false, 0)
        };

        Some(SkillInfo {
            id,
            name,
            description,
            has_scripts,
            script_count,
            bom_clean,
        })
    }

    /// 扫描当前项目实际安装的技能目录（仅扫描项目自身的 `.y3maker/skills`、`.agents/skills`、`.claude/skills`）
    pub fn scan_skills(y3maker_dir: &Path, project_path: &Path) -> Vec<SkillInfo> {
        let candidates = [
            y3maker_dir.join("skills"),
            project_path.join(".agents").join("skills"),
            project_path.join(".claude").join("skills"),
        ];

        for dir in candidates {
            if dir.is_dir() {
                let mut list = Vec::new();
                if let Ok(rd) = fs::read_dir(&dir) {
                    for entry in rd.flatten() {
                        let p = entry.path();
                        if p.is_dir() {
                            if let Some(info) = Self::parse_skill_md(&p) {
                                list.push(info);
                            }
                        }
                    }
                }
                if !list.is_empty() {
                    list.sort_by(|a, b| a.id.cmp(&b.id));
                    return list;
                }
            }
        }
        Vec::new()
    }

    /// 读取单个技能的完整 `SKILL.md` 文档与脚本清单（供操作台点击查看详情）
    pub fn get_skill_detail(
        project: &ProjectContext,
        skill_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let project_path = &project.project_path;
        let candidates = [
            project_path.join(".y3maker").join("skills").join(skill_id),
            project_path.join(".agents").join("skills").join(skill_id),
            Self::app_global_hub_dir().join("skills").join(skill_id),
        ];
        for dir in candidates {
            let md = dir.join("SKILL.md");
            if md.is_file() {
                let raw = fs::read(&md)?;
                let clean = if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
                    &raw[3..]
                } else {
                    &raw[..]
                };
                let content = String::from_utf8_lossy(clean).to_string();
                let mut scripts = Vec::new();
                let s_dir = dir.join("scripts");
                if s_dir.is_dir() {
                    for entry in WalkDir::new(&s_dir).min_depth(1).into_iter().flatten() {
                        if entry.file_type().is_file() {
                            if let Ok(rel) = entry.path().strip_prefix(&dir) {
                                scripts.push(rel.to_string_lossy().to_string());
                            }
                        }
                    }
                }
                return Ok(json!({
                    "id": skill_id,
                    "path": dir.to_string_lossy(),
                    "content": content,
                    "scripts": scripts
                }));
            }
        }
        anyhow::bail!("未找到技能 {} 的 SKILL.md 文件", skill_id)
    }

    fn count_files_in(dir: &Path) -> usize {
        if !dir.is_dir() {
            return 0;
        }
        WalkDir::new(dir)
            .follow_links(true)
            .min_depth(1)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .count()
    }

    /// 全面检查当前项目的 `.y3maker` 技能库状态、全局缓存母库以及各主流 AI 智能体的标准装配状态
    pub fn inspect_hub_status(project: &ProjectContext, mcp_port: u16) -> Y3MakerHubStatus {
        let project_path = &project.project_path;
        let y3maker_dir = project_path.join(".y3maker");
        let status = Self::inspect_y3maker_status(project_path);

        let (git_commit, git_commit_time, git_commit_msg, git_remote) =
            Self::read_git_head_meta(&y3maker_dir);

        let global_hub = Self::app_global_hub_dir();
        let global_hub_ready = global_hub.join("skills").is_dir();
        let (global_hub_commit, _, _, _) = Self::read_git_head_meta(&global_hub);

        let skills = Self::scan_skills(&y3maker_dir, project_path);
        let knowledge_count = Self::count_files_in(&y3maker_dir.join("knowledge"));
        let templates_count = Self::count_files_in(&y3maker_dir.join("templates"));
        let tools_count = Self::count_files_in(&y3maker_dir.join("tools"));

        let agents_skills_dir = project_path.join(".agents").join("skills");
        let agents_skills_linked =
            agents_skills_dir.is_dir() && Self::count_files_in(&agents_skills_dir) > 0;

        let claude_skills_dir = project_path.join(".claude").join("skills");
        let claude_skills_linked =
            claude_skills_dir.is_dir() && Self::count_files_in(&claude_skills_dir) > 0;

        let has_agents_md = project_path.join("AGENTS.md").is_file();

        // 检查是否已完成 Y3 Studio MCP 适配转换
        let bridge_md = y3maker_dir.join("rules").join("y3-studio-mcp-bridge.md");
        let mcp_settings = y3maker_dir.join("mcp_settings.json");
        let mcp_adapted = bridge_md.is_file() && mcp_settings.is_file();
        let mcp_adapter_notes = if mcp_adapted {
            vec![
                format!("1. 双格式 .y3maker/mcp_settings.json 已适配 (同时兼容 mcp_batch_writer.py 与 mcp_entity_writer.py，指向 :{} 网关)", mcp_port),
                "2. y3-auto-test 已适配常驻 y3runtime 网关（启动游戏无需暂停提醒用户手动刷新 MCP）".to_string(),
                "3. 服务名双别名已启用：同时暴露 y3-helper 与 helper，100% 兼容官方技能中的 helper.xxx 调用".to_string(),
                "4. 已注入 y3-studio-mcp-bridge.md 多项目路由与 .codemaker 路径映射规则".to_string(),
            ]
        } else {
            vec![
                "官方原版 .y3maker 默认写死 8765/8766/25000 端口、混用 helper/y3-helper 名称且要求手动刷新 y3runtime；点击「拉取/同步」或「一键装配」将自动完成 Y3 Studio 网关适配。".to_string(),
            ]
        };

        // 标准主流 AI 编程智能体装配状态检测
        let mut agent_targets = Vec::new();
        let home = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default();

        // 1. 通用 Agent 开放标准 (.agents/skills + AGENTS.md + .mcp.json)
        let proj_mcp = project_path.join(".mcp.json");
        let proj_equipped = agents_skills_linked && has_agents_md && proj_mcp.is_file();
        agent_targets.push(AgentTargetStatus {
            id: "project_links".to_string(),
            name: "通用 Agent 标准 (.agents/skills)".to_string(),
            scope: "project".to_string(),
            config_path: format!(
                "{} & {}",
                agents_skills_dir.display(),
                proj_mcp.display()
            ),
            detected: true,
            equipped: proj_equipped,
            detail: String::new(),
        });

        // 2. Claude Code
        let claude_md = project_path.join("CLAUDE.md");
        let claude_equipped = claude_skills_linked && proj_mcp.is_file();
        let _ = claude_md;
        agent_targets.push(AgentTargetStatus {
            id: "claude".to_string(),
            name: "Claude Code (.claude/skills)".to_string(),
            scope: "project".to_string(),
            config_path: claude_skills_dir.to_string_lossy().to_string(),
            detected: true,
            equipped: claude_equipped,
            detail: String::new(),
        });

        // 3. Cursor IDE
        let cursor_mcp = project_path.join(".cursor").join("mcp.json");
        let cursor_rule = project_path
            .join(".cursor")
            .join("rules")
            .join("y3-helper.mdc");
        let cursor_equipped = cursor_mcp.is_file() && cursor_rule.is_file();
        agent_targets.push(AgentTargetStatus {
            id: "cursor".to_string(),
            name: "Cursor IDE (.cursor/mcp.json)".to_string(),
            scope: "project".to_string(),
            config_path: cursor_mcp.to_string_lossy().to_string(),
            detected: true,
            equipped: cursor_equipped,
            detail: String::new(),
        });

        // 4. Windsurf / Cline
        let windsurf_rules = project_path.join(".windsurfrules");
        let windsurf_equipped = windsurf_rules.is_file() && proj_mcp.is_file();
        agent_targets.push(AgentTargetStatus {
            id: "windsurf".to_string(),
            name: "Windsurf / Cline (.windsurfrules)".to_string(),
            scope: "project".to_string(),
            config_path: windsurf_rules.to_string_lossy().to_string(),
            detected: false,
            equipped: windsurf_equipped,
            detail: String::new(),
        });

        // 5. OpenAI Codex
        let codex_cfg = home.join(".codex").join("config.toml");
        let codex_detected = codex_cfg.is_file() || home.join(".codex").is_dir();
        let codex_content = fs::read_to_string(&codex_cfg).unwrap_or_default();
        let codex_has_mcp = codex_content.contains("[mcp_servers.y3-helper]")
            || codex_content.contains("[mcp_servers.\"y3-helper\"]");
        agent_targets.push(AgentTargetStatus {
            id: "codex".to_string(),
            name: "OpenAI Codex (~/.codex/config.toml)".to_string(),
            scope: "global".to_string(),
            config_path: codex_cfg.to_string_lossy().to_string(),
            detected: codex_detected,
            equipped: codex_has_mcp && agents_skills_linked,
            detail: String::new(),
        });

        Y3MakerHubStatus {
            project_id: project.project_id.clone(),
            project_path: project_path.to_string_lossy().to_string(),
            y3maker_dir: y3maker_dir.to_string_lossy().to_string(),
            status: if y3maker_dir.join("skills").is_dir() && status == "missing" {
                "partial".to_string()
            } else {
                status
            },
            git_commit,
            git_commit_msg,
            git_commit_time,
            git_remote,
            global_hub_dir: global_hub.to_string_lossy().to_string(),
            global_hub_ready,
            global_hub_commit,
            skills_count: skills.len(),
            knowledge_count,
            templates_count,
            tools_count,
            agents_skills_linked,
            claude_skills_linked,
            has_agents_md,
            mcp_adapted,
            mcp_adapter_notes,
            skills,
            agent_targets,
        }
    }

    /// 一键从官方 Git 仓库（Gitee / GitHub / 自定义 URL / 全局母库缓存）拉取或更新全局母库与当前项目 `.y3maker`
    pub fn sync_y3maker_hub(
        project: &ProjectContext,
        source_str: Option<&str>,
        mcp_port: u16,
    ) -> anyhow::Result<serde_json::Value> {
        let project_path = &project.project_path;
        let target_dir = project_path.join(".y3maker");
        let skip_network = matches!(
            source_str.map(|s| s.trim().to_lowercase()).as_deref(),
            Some("local") | Some("hub") | Some("cache")
        );
        let remote_url = Self::resolve_y3maker_git_url(source_str);

        let action_result =
            Self::sync_project_from_hub_or_remote(project_path, &remote_url, skip_network)?;
        let bom_fixed = Self::strip_skills_utf8_bom(&target_dir);
        let adapter_notes =
            Self::adapt_y3maker_mcp_for_studio(&target_dir, project_path, mcp_port)
                .unwrap_or_default();

        // 自动确保 .agents/skills 指向 .y3maker/skills
        let skills_target = target_dir.join("skills");
        if skills_target.is_dir() {
            let _ = Self::ensure_dir_junction(
                &project_path.join(".agents").join("skills"),
                &skills_target,
            );
        }

        let hub = Self::inspect_hub_status(project, mcp_port);
        Ok(json!({
            "success": true,
            "action": action_result,
            "remote_url": remote_url,
            "bom_fixed_files": bom_fixed,
            "adapter_notes": adapter_notes,
            "hub": hub
        }))
    }

    /// 一键将 `.y3maker` 技能库、开发规范与三合一 MCP 网关装配到指定智能体
    pub fn equip_agents(
        project: &ProjectContext,
        targets: &[String],
        mcp_port: u16,
    ) -> anyhow::Result<serde_json::Value> {
        let project_path = &project.project_path;
        let y3maker_dir = project_path.join(".y3maker");

        // 1. 如果项目还没有 .y3maker/skills 或技能文件为空，优先从客户端全局母库毫秒级分发
        if !y3maker_dir.join("skills").is_dir()
            || Self::count_files_in(&y3maker_dir.join("skills")) == 0
        {
            let has_global_cache = Self::app_global_hub_dir().join("skills").is_dir();
            let _ = Self::sync_project_from_hub_or_remote(
                project_path,
                Y3MAKER_REPO_GITEE,
                has_global_cache,
            );
        }
        let bom_fixed = Self::strip_skills_utf8_bom(&y3maker_dir);
        let adapter_notes =
            Self::adapt_y3maker_mcp_for_studio(&y3maker_dir, project_path, mcp_port)
                .unwrap_or_default();
        let skills_src = y3maker_dir.join("skills");

        let selected: Vec<String> = if targets.is_empty() {
            vec![
                "project_links".to_string(),
                "claude".to_string(),
                "cursor".to_string(),
                "codex".to_string(),
            ]
        } else {
            targets.to_vec()
        };

        let helper_url = format!("http://127.0.0.1:{}/mcp", mcp_port);
        let editor_url = format!("http://127.0.0.1:{}/y3editor/mcp", mcp_port);
        let runtime_url = format!("http://127.0.0.1:{}/y3runtime/mcp", mcp_port);

        let mcp_json_payload = json!({
            "mcpServers": {
                "y3-helper": {
                    "type": "http",
                    "url": helper_url
                },
                "y3editor": {
                    "type": "http",
                    "url": editor_url
                },
                "y3runtime": {
                    "type": "http",
                    "url": runtime_url
                }
            }
        });

        let mut logs = Vec::new();
        logs.extend(adapter_notes);
        let home = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default();

        for t in &selected {
            match t.as_str() {
                "project_links" => {
                    if skills_src.is_dir() {
                        let link_path = project_path.join(".agents").join("skills");
                        let _ = Self::ensure_dir_junction(&link_path, &skills_src);
                        logs.push(format!("已链接项目技能目录: {}", link_path.display()));
                    }
                    let agents_md = project_path.join("AGENTS.md");
                    if !agents_md.exists() {
                        let tpl = y3maker_dir.join("rules").join("AGENTS.md");
                        if tpl.is_file() {
                            let _ = fs::copy(&tpl, &agents_md);
                        } else {
                            let _ = fs::write(
                                &agents_md,
                                "# Y3 地图工程开发规范\n\n请遵循 `.y3maker/rules/AGENTS.md` 与 `.y3maker/rules/y3-studio-mcp-bridge.md` 规范，按需加载 `.agents/skills/` 下的官方技能。\n",
                            );
                        }
                        logs.push("已生成项目 AGENTS.md".to_string());
                    } else {
                        logs.push("已保留项目现有的自定义 AGENTS.md（未覆盖）".to_string());
                    }
                    let proj_mcp = project_path.join(".mcp.json");
                    fs::write(&proj_mcp, serde_json::to_string_pretty(&mcp_json_payload)?)?;
                    logs.push(format!("已写入项目级 MCP 配置: {}", proj_mcp.display()));
                }
                "claude" => {
                    if skills_src.is_dir() {
                        let link_path = project_path.join(".claude").join("skills");
                        let _ = Self::ensure_dir_junction(&link_path, &skills_src);
                        logs.push(format!("已建立 Claude Code 技能链接: {}", link_path.display()));
                    }
                    let claude_md = project_path.join("CLAUDE.md");
                    if !claude_md.exists() {
                        let content = "# Claude Code Project Instructions\n\n请遵循本项目根目录下的 `AGENTS.md`、`.y3maker/rules/AGENTS.md` 以及 `.y3maker/rules/y3-studio-mcp-bridge.md` 中的全部 Y3 开发规范，按任务调用 `.claude/skills/` 下的官方技能。\n";
                        let _ = fs::write(&claude_md, content);
                        logs.push("已生成 CLAUDE.md 引导文件".to_string());
                    }
                    let proj_mcp = project_path.join(".mcp.json");
                    fs::write(&proj_mcp, serde_json::to_string_pretty(&mcp_json_payload)?)?;
                    logs.push(format!("已更新 Claude Code 项目级 MCP: {}", proj_mcp.display()));
                }
                "cursor" => {
                    let cursor_dir = project_path.join(".cursor");
                    let rules_dir = cursor_dir.join("rules");
                    fs::create_dir_all(&rules_dir)?;
                    let cursor_mcp = cursor_dir.join("mcp.json");
                    fs::write(&cursor_mcp, serde_json::to_string_pretty(&mcp_json_payload)?)?;
                    let mdc_path = rules_dir.join("y3-helper.mdc");
                    let mdc_content = "---\ndescription: Y3 Editor Map Development Rules & Official Skills Router (.y3maker)\nglobs: **/*\nalwaysApply: true\n---\n\n请遵循项目根目录 `AGENTS.md`、`.y3maker/rules/AGENTS.md` 与 `.y3maker/rules/y3-studio-mcp-bridge.md` 开发规范。所有 Y3 官方技能位于 `.y3maker/skills/<skill-name>/SKILL.md`，知识库位于 `.y3maker/knowledge/`。\n";
                    fs::write(&mdc_path, mdc_content)?;
                    logs.push(format!(
                        "已生成 Cursor MCP 配置与 .mdc 规则: {}",
                        cursor_mcp.display()
                    ));
                }
                "windsurf" => {
                    let ws_rules = project_path.join(".windsurfrules");
                    if !ws_rules.exists() {
                        let content = "# Y3 Editor Development Rules\n\n请遵循项目根目录下 `AGENTS.md`、`.y3maker/rules/AGENTS.md` 与 `.y3maker/rules/y3-studio-mcp-bridge.md` 的规范，按任务读取 `.y3maker/skills/<skill-name>/SKILL.md`。\n";
                        fs::write(&ws_rules, content)?;
                    }
                    let proj_mcp = project_path.join(".mcp.json");
                    fs::write(&proj_mcp, serde_json::to_string_pretty(&mcp_json_payload)?)?;
                    logs.push(format!("已生成 .windsurfrules 与 .mcp.json: {}", ws_rules.display()));
                }
                "codex" => {
                    if skills_src.is_dir() {
                        let link_path = project_path.join(".agents").join("skills");
                        let _ = Self::ensure_dir_junction(&link_path, &skills_src);
                    }
                    let codex_dir = home.join(".codex");
                    fs::create_dir_all(&codex_dir)?;
                    let codex_cfg = codex_dir.join("config.toml");
                    let existing = fs::read_to_string(&codex_cfg).unwrap_or_default();
                    let updated = Self::upsert_codex_config_toml(
                        &existing,
                        &helper_url,
                        &editor_url,
                        &runtime_url,
                        &project_path.to_string_lossy(),
                    );
                    fs::write(&codex_cfg, updated)?;
                    logs.push(format!(
                        "已更新 OpenAI Codex 全局配置: {}",
                        codex_cfg.display()
                    ));
                }
                _ => {}
            }
        }

        let hub = Self::inspect_hub_status(project, mcp_port);
        Ok(json!({
            "success": true,
            "bom_fixed_files": bom_fixed,
            "equipped_targets": selected,
            "logs": logs,
            "hub": hub
        }))
    }

    /// 无损更新 `~/.codex/config.toml` 中的 `[mcp_servers.y3-helper]`、`[mcp_servers.y3editor]`、`[mcp_servers.y3runtime]`
    fn upsert_codex_config_toml(
        existing: &str,
        helper_url: &str,
        editor_url: &str,
        runtime_url: &str,
        project_path: &str,
    ) -> String {
        let mut kept_lines = Vec::new();
        let mut skip_section = false;
        for line in existing.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let sec = trimmed.to_lowercase();
                if sec.starts_with("[mcp_servers.y3-helper")
                    || sec.starts_with("[mcp_servers.\"y3-helper\"")
                    || sec.starts_with("[mcp_servers.helper]")
                    || sec.starts_with("[mcp_servers.\"helper\"]")
                    || sec.starts_with("[mcp_servers.y3editor")
                    || sec.starts_with("[mcp_servers.\"y3editor\"")
                    || sec.starts_with("[mcp_servers.y3runtime")
                    || sec.starts_with("[mcp_servers.\"y3runtime\"")
                {
                    skip_section = true;
                    continue;
                } else {
                    skip_section = false;
                }
            }
            if !skip_section {
                kept_lines.push(line);
            }
        }

        let mut out = kept_lines.join("\n").trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "[mcp_servers.y3-helper]\nurl = \"{}\"\n\n[mcp_servers.y3editor]\nurl = \"{}\"\n\n[mcp_servers.y3runtime]\nurl = \"{}\"\n",
            helper_url, editor_url, runtime_url
        ));

        let proj_header = format!("[projects.'{}']", project_path);
        if !out.contains(&proj_header) {
            out.push_str(&format!("\n{}\ntrust_level = \"trusted\"\n", proj_header));
        }
        out
    }
}
