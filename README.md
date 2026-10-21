# Y3 Studio (`y3-helper` 原生独立版)

一个用 **纯 Rust** 写的 Y3 编辑器开发助手与 AI MCP 常驻网关。

不需要安装 VSCode、不需要配 Node.js 环境，双击一个 `y3-helper.exe`，就能把 **Y3 地图日常调试**、**`.y3maker` AI 技能库装配** 和 **任何 AI 编程工具（Cursor / Claude Code / Windsurf / Antigravity）** 全部打通。

---

## 为什么做这个？（相比官方插件有什么好处）

### 1. 彻底摆脱 VSCode 依赖，什么 AI 软件都能直接用
官方的 `y3-helper` 是一个必须活在 VSCode 里的扩展插件。如果你平时习惯用 **Cursor、Windsurf、Claude Code、Antigravity**，或者根本不想开笨重的 VSCode，这个独立客户端能让你在任何编辑器/AI 工具下直接使用完整的 Y3 工具链。

### 2. 内存占用只有几 MB，挂后台完全无感
界面采用纯 Rust 原生硬件渲染（`eframe` / `egui`），**0 WebView2 进程、0 Electron、0 Node.js**。
- 正常打开界面仅占 **15~25 MB** 内存；
- 点击右上角 `×` 隐藏到右下角系统托盘后，后台驻留内存只有 **3~5 MB**。

### 3. 三合一常驻 MCP 网关，重启游戏再也不用手动点 Refresh
以前用官方的 `y3runtime` MCP，只要你关掉游戏或者重启一局，MCP 连接就会断开，每次都得回 AI 软件里手动点一下刷新重连，非常打断思路。

**Y3 Studio** 在一个固定端口（默认 `8766`）同时托管了 3 个 MCP 端点：
- `/mcp`（`y3-helper` / `helper`：启动游戏、热重载 `.rd`、物编读写、ECA 编译、读日志）
- `/y3editor/mcp`（`y3editor`：自动发现并转发给当前打开的 Y3 编辑器）
- `/y3runtime/mcp`（`y3runtime`：常驻在线！游戏没开时温柔提示 AI，游戏一开自动接管，**配一次终身免刷新**）

### 4. 一键配好 `.y3maker` 技能库，不用自己敲命令建软链接
内置全局技能母库缓存。不管是新建地图还是老地图接入 AI，点一下 **「同步技能库」** + **「一键装配」**，1 秒内自动帮你搞定：
- 11 个官方 Y3 核心开发技能（游戏策划、Lua 管线、物编修改、UI 生成、自动化测试等）
- 知识库、错题本、功能模板与 Python 辅助脚本
- 自动创建 `.agents/skills`、`.claude/skills` 目录链接，并把技能里的旧版 MCP 前缀自动适配到当前网关

### 5. 支持多项目、多子地图同时挂载，绝不误杀
左侧栏可以同时添加多个 Y3 地图项目，每个项目分配独立的调试端口：
- 随时点击切换当前激活的项目或子地图（如 `EntryMap`）；
- 点「结束游戏」时只关闭当前项目的游戏窗口，绝不误杀电脑上开着的其他项目或编辑器。

---

## 怎么用？（3 步上手）

### 第一步：添加并初始化你的 Y3 地图项目
打开 `y3-helper.exe`，点击左上角 **`+ 添加项目`**，选择你的 Y3 地图根目录（里面有 `header.project` 或 `header.map` 的那个文件夹）。如果是新建的地图，直接点击顶部快捷栏的 **「初始化项目」**（或技能库页面的 **「初始化项目 (y3-lualib)」**），会自动克隆 `y3-lualib` 到 `script/y3`、复制模板项目配置并同步 `.y3maker`。

### 第二步：同步并一键装配 AI 技能
在默认打开的 **「技能库 (.y3maker)」** 页面：
1. 点击左侧卡片的 **「同步技能库」**（或 **「初始化项目 (y3-lualib)」**，首次拉取后会自动缓存在本机，以后新建项目都是秒同步）；
2. 在右侧卡片勾选你正在使用的 AI 工具，点击 **「一键装配」**，状态全部变成绿色的 **`已装配`** 就行了。

### 第三步：复制 MCP 配置到你的 AI 软件
点击窗口左下角的 **「复制 MCP 配置」** 按钮，它会把下面这段配置复制到剪贴板，粘贴进你的 AI 编程工具（如 Cursor / Windsurf / Claude / Antigravity 的 `mcp.json`）里保存即可：

```json
{
  "mcpServers": {
    "y3-helper": { "url": "http://127.0.0.1:8766/mcp" },
    "y3editor": { "url": "http://127.0.0.1:8766/y3editor/mcp" },
    "y3runtime": { "url": "http://127.0.0.1:8766/y3runtime/mcp" }
  }
}
```

配好之后，直接点窗口右上角的 **`×`** 把它收进右下角系统托盘，就可以在 AI 软件里愉快地让 AI 帮你做地图、写 Lua、改物编、自动启动游戏跑测试了！

---

## 界面功能一览

- **顶部快捷栏**：
  - `▶ 启动游戏` / `打开编辑器` / `初始化项目`（新地图一键克隆 `y3-lualib` 到 `script/y3`、复制项目配置并初始化 `.y3maker`）
  - `热重载 (.rd)`（改完 Lua 不用重启游戏直接生效）
  - `物编热更` / `触发器热更` / `重启 (.rr)` / `结束游戏` / `打开目录`
- **技能库 (.y3maker)**：管理官方技能库同步、AI 工具装配状态，点击任意技能卡片可直接查看技能说明文档。
- **调试与日志**：
  - 往正在运行的游戏里直接发一段 Lua 代码执行（比如打印变量、查看 `collectgarbage` 内存）；
  - 输入 UI 路径模拟游戏内鼠标左/右键点击；
  - 实时拉取游戏运行日志。
- **物编与导表**：按分类（单位 / 技能 / 魔法效果 / 物品 / 投射物 / 可破坏物 / 装饰物）搜索、查看和修改物编 JSON，自动同步中文语言表（`zhlanguage.json`）。
- **编译与打包**：编译 ECA 触发器为 Lua 代码、生成代码补全用的 `Meta` 和 `LuaDoc`、开启全局脚本、一键打包 `.gmp` 文件。
- **多项目与网关**：一键扫描电脑上正在运行的 `Editor.exe` 进程及各自的 MCP 端口。
- **系统托盘**：
  - 点击右上角 `×` 隐藏到托盘（保持后台 MCP 网关不断线）；
  - **左键**点击托盘图标恢复主窗口，**右键**点击托盘图标可选择彻底退出。

---

## 命令行模式（无界面用法）

除了双击打开桌面界面，`y3-helper.exe` 也支持纯命令行调用（适合放进自动化脚本或 CI）：

```powershell
# 启动无界面的纯后台 HTTP MCP 网关服务
y3-helper.exe serve --port 8766 --project "D:\Maps\MyMap"

# 启动 Stdio 模式的 MCP 服务
y3-helper.exe mcp-stdio --project "D:\Maps\MyMap"

# 常用单次命令
y3-helper.exe launch-game -p "D:\Maps\MyMap" --multi --players 1,2
y3-helper.exe reload-lua -p "D:\Maps\MyMap"
y3-helper.exe compile-eca -p "D:\Maps\MyMap"
y3-helper.exe generate-meta -p "D:\Maps\MyMap"
y3-helper.exe save-gmp -p "D:\Maps\MyMap"
```

---

## 从源码编译与打包发行版

环境要求：Windows + Rust 工具链（`cargo`）

```powershell
# GitHub 源
git clone https://github.com/xiaobeiswift/y3-studio.git
# 或 Gitee 国内源
git clone https://gitee.com/xiaobeiswift/y3-studio.git

cd y3-studio

# 仅编译主程序
cargo build --release

# 一键构建正式版（同时生成 Windows 安装包 setup.exe 与绿色包 portable.zip）
powershell -ExecutionPolicy Bypass -File .\scripts\build-release.ps1
```

打包完成后产物位于 `dist\` 目录：
- `dist\Y3-Studio-v0.0.1-win-x64-setup.exe`：Windows 图形化安装包（含桌面/开始菜单快捷方式、PATH 环境变量配置及标准卸载程序 `uninstall.exe`，支持 `/S` 静默安装）
- `dist\Y3-Studio-v0.0.1-win-x64-portable.zip`：Windows 绿色免安装包（解压即用，单文件无外部 DLL 依赖）
