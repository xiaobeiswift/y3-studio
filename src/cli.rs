use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "y3-helper",
    version,
    about = "Y3-Helper 独立 Rust 应用（剥离 VS Code UI，支持同一台机器多项目、多地图同时并发操作）"
)]
pub struct Cli {
    /// 手动指定 Y3 Editor.exe 路径（可选，默认自动从注册表或 Y3_EDITOR_PATH 探测）
    #[arg(long, global = true)]
    pub editor_path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 启动独立桌面图形客户端界面（默认无参数双击运行时自动进入此模式）
    Gui {
        /// 本地服务端口（默认 8766）
        #[arg(short, long, default_value_t = 8766)]
        port: u16,
        /// 启动时预挂载的一个或多个 Y3 项目路径
        #[arg(long = "project", short = 'P')]
        projects: Vec<PathBuf>,
    },

    /// 启动多项目 MCP Streamable HTTP + REST API 守护服务 (默认端口 8766)
    Serve {
        /// HTTP 监听端口
        #[arg(short, long, default_value_t = 8766)]
        port: u16,
        /// 启动时预挂载的一个或多个 Y3 项目路径（可多次指定 `--project A --project B`）
        #[arg(long = "project", short = 'P')]
        projects: Vec<PathBuf>,
        /// 是否同时启动 25895/25896 网络测试服务器
        #[arg(long)]
        network_test_server: bool,
    },

    /// 启动 MCP Stdio 模式服务（供 AI 客户端通过标准输入输出直接连接，支持多项目并发操作）
    McpStdio {
        /// 预挂载的一个或多个 Y3 项目路径
        #[arg(long = "project", short = 'P')]
        projects: Vec<PathBuf>,
    },

    /// 查看本机 Y3 编辑器环境与指定项目（支持多个项目）的子地图状态
    Status {
        /// 要检查的一个或多个 Y3 项目路径
        #[arg(long = "project", short = 'P')]
        projects: Vec<PathBuf>,
    },

    /// 初始化 Y3 地图工程（克隆 y3-lualib、复制模板配置、克隆 .y3maker）
    Init {
        /// 项目或地图路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 指定子地图名称（默认当前地图/EntryMap）
        #[arg(long, short = 'm')]
        map: Option<String>,
        /// 代码源 (gitee | github | english)
        #[arg(long, default_value = "gitee")]
        source: String,
    },

    /// 启动指定项目、指定子地图的游戏
    LaunchGame {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称
        #[arg(long, short = 'm')]
        map: Option<String>,
        /// 是否启用多开模式
        #[arg(long)]
        multi: bool,
        /// 多开玩家编号列表（如 `--players 1,2`）
        #[arg(long, value_delimiter = ',')]
        players: Option<Vec<u32>>,
        /// 是否启动 Tracy
        #[arg(long)]
        tracy: bool,
    },

    /// 在 Y3 编辑器中打开指定项目
    LaunchEditor {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
    },

    /// 终止指定项目的游戏进程（或通过 --all 终止所有游戏进程）
    KillGame {
        /// 项目路径
        #[arg(long, short = 'p')]
        project: Option<PathBuf>,
        /// 终止本机所有游戏模式的 Game_x64h.exe
        #[arg(long)]
        all: bool,
    },

    /// 将指定项目、指定子地图的 ECA 触发器编译为 Lua 代码
    CompileEca {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称
        #[arg(long, short = 'm')]
        map: Option<String>,
        /// 可选的触发器 Excel 覆盖目录
        #[arg(long)]
        override_dir: Option<PathBuf>,
    },

    /// 为指定项目生成自定义枚举 Meta 代码 (unitAttrs, playerAttrs, customEvents, ui, objects 等)
    GenerateMeta {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称（不传则为该项目下所有子地图生成）
        #[arg(long, short = 'm')]
        map: Option<String>,
    },

    /// 重建并保存指定地图的二进制 .gmp 文件（整合 editor_table/, ui/, 语言 JSON）
    SaveGmp {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称
        #[arg(long, short = 'm')]
        map: Option<String>,
    },

    /// 物编表查询、创建、修改与删除
    Table {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称
        #[arg(long, short = 'm')]
        map: Option<String>,
        /// 物编表类型 (单位/unit, 技能/ability, 魔法效果/modifier, 物品/item, 投射物/projectile, 可破坏物/destructible, 装饰物/decoration, 声音/sound, 科技/technology)
        #[arg(long, short = 't', default_value = "unit")]
        table_type: String,
        /// 操作类型 (list | get | upsert | delete)
        #[arg(long, short = 'a', default_value = "list")]
        action: String,
        /// 物编 key
        #[arg(long, short = 'k')]
        key: Option<u64>,
        /// 物编名称 (upsert 时使用)
        #[arg(long, short = 'n')]
        name: Option<String>,
        /// JSON 格式的字段修改内容 (如 `{"ori_bits": 10}`)
        #[arg(long)]
        fields_json: Option<String>,
        /// JSON 格式的自定义 kv 修改内容
        #[arg(long)]
        kv_json: Option<String>,
    },

    /// 将 Excel (.xlsx) 表格批量导入到物编表
    ExcelImport {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称
        #[arg(long, short = 'm')]
        map: Option<String>,
        /// 目标物编表类型
        #[arg(long, short = 't', default_value = "unit")]
        table_type: String,
        /// Excel 文件路径
        #[arg(long, short = 'e')]
        excel: String,
        /// 工作表名称
        #[arg(long, short = 's')]
        sheet: Option<String>,
    },

    /// 为指定项目启用 global_script 全局脚本功能
    GlobalScript {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
    },

    /// 启用或禁用本地云脚本调试引导 (cloud_script/main.lua)
    CloudScript {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 是否启用
        #[arg(long, default_value_t = true)]
        enable: bool,
        /// 是否等待调试器连接
        #[arg(long)]
        wait_debugger: bool,
    },

    /// 管理并运行项目的 y3-helper JS 插件
    Plugin {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称
        #[arg(long, short = 'm')]
        map: Option<String>,
        /// 操作 (init | list | run)
        #[arg(long, short = 'a', default_value = "list")]
        action: String,
        /// 要运行的函数名 (如 onSave, onGame, onEditor)
        #[arg(long, default_value = "onSave")]
        func: String,
    },

    /// 导出并查看指定地图的 UI 画布节点树
    UiCanvas {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称
        #[arg(long, short = 'm')]
        map: Option<String>,
        /// 节点路径 (如 MainHUD.SkillBar)
        #[arg(long)]
        node_path: Option<String>,
        /// 展开深度 (-1 表示无限深度)
        #[arg(long, default_value_t = -1)]
        depth: i64,
    },

    /// 从 doc.json 生成 y3/doc/API.md 文档
    LuaDoc {
        /// 项目路径
        #[arg(long, short = 'p', default_value = ".")]
        project: PathBuf,
        /// 子地图名称
        #[arg(long, short = 'm')]
        map: Option<String>,
    },

    /// 启动 Y3 网络库测试服务器 (127.0.0.1:25895 & 25896)
    NetworkServer,

    /// 内置免 UAC 提权代理进程模式
    Agent {
        /// 允许启动的 Game_x64h.exe 路径
        #[arg(long)]
        allowed_exe: PathBuf,
    },
}
