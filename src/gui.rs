use crate::features::project_init::{ProjectInitializer, Y3MakerHubStatus};
use crate::server::ToolDispatcher;
use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, Frame, Margin, RichText, Rounding,
    ScrollArea, Stroke, TextEdit, Vec2,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicIsize, Ordering},
    mpsc, Arc,
};

#[cfg(windows)]
static MAIN_WINDOW_HWND: AtomicIsize = AtomicIsize::new(0);

#[cfg(windows)]
#[repr(C)]
#[derive(Copy, Clone)]
struct Point {
    x: i32,
    y: i32,
}

#[cfg(windows)]
#[repr(C)]
struct Msg {
    hwnd: isize,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: Point,
}

#[cfg(windows)]
#[repr(C)]
struct WndClassW {
    style: u32,
    lpfn_wnd_proc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: isize,
    h_icon: isize,
    h_cursor: isize,
    hbr_background: isize,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
}

#[cfg(windows)]
#[repr(C)]
struct NotifyIconDataW {
    cb_size: u32,
    h_wnd: isize,
    u_id: u32,
    u_flags: u32,
    u_callback_message: u32,
    h_icon: isize,
    sz_tip: [u16; 128],
    dw_state: u32,
    dw_state_mask: u32,
    sz_info: [u16; 256],
    u_timeout_or_version: u32,
    sz_info_title: [u16; 64],
    dw_info_flags: u32,
    guid_item: [u8; 16],
    h_balloon_icon: isize,
}

#[cfg(windows)]
extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn K32EmptyWorkingSet(hProcess: *mut std::ffi::c_void) -> i32;

    fn GetModuleHandleW(lpModuleName: *const u16) -> isize;
    fn RegisterClassW(lpWndClass: *const WndClassW) -> u16;
    fn CreateWindowExW(
        dwExStyle: u32,
        lpClassName: *const u16,
        lpWindowName: *const u16,
        dwStyle: u32,
        X: i32,
        Y: i32,
        nWidth: i32,
        nHeight: i32,
        hWndParent: isize,
        hMenu: isize,
        hInstance: isize,
        lpParam: *const std::ffi::c_void,
    ) -> isize;
    fn DefWindowProcW(hWnd: isize, Msg: u32, wParam: usize, lParam: isize) -> isize;
    fn GetMessageW(lpMsg: *mut Msg, hWnd: isize, wMsgFilterMin: u32, wMsgFilterMax: u32) -> i32;
    fn TranslateMessage(lpMsg: *const Msg) -> i32;
    fn DispatchMessageW(lpMsg: *const Msg) -> isize;

    fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> isize;
    fn GetActiveWindow() -> isize;
    fn ShowWindow(hWnd: isize, nCmdShow: i32) -> i32;
    fn SetForegroundWindow(hWnd: isize) -> i32;

    fn CreateIcon(
        hInstance: isize,
        nWidth: i32,
        nHeight: i32,
        cPlanes: u8,
        cBitsPixel: u8,
        lpbANDbits: *const u8,
        lpbXORbits: *const u8,
    ) -> isize;
    fn Shell_NotifyIconW(dwMessage: u32, lpData: *const NotifyIconDataW) -> i32;

    fn CreatePopupMenu() -> isize;
    fn AppendMenuW(hMenu: isize, uFlags: u32, uIDNewItem: usize, lpNewItem: *const u16) -> i32;
    fn DestroyMenu(hMenu: isize) -> i32;
    fn GetCursorPos(lpPoint: *mut Point) -> i32;
    fn TrackPopupMenu(
        hMenu: isize,
        uFlags: u32,
        x: i32,
        y: i32,
        nReserved: i32,
        hWnd: isize,
        prcRect: *const std::ffi::c_void,
    ) -> i32;
}

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn ensure_main_hwnd() -> isize {
    let cached = MAIN_WINDOW_HWND.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    unsafe {
        let title = to_wide("Y3 Studio");
        let mut hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd == 0 {
            hwnd = GetActiveWindow();
        }
        if hwnd != 0 {
            MAIN_WINDOW_HWND.store(hwnd, Ordering::Relaxed);
        }
        hwnd
    }
}

#[cfg(windows)]
fn hide_main_window_to_tray() {
    let hwnd = ensure_main_hwnd();
    if hwnd != 0 {
        unsafe {
            // SW_HIDE = 0
            let _ = ShowWindow(hwnd, 0);
        }
    }
    trim_self_working_set();
}

#[cfg(windows)]
fn restore_main_window_from_tray() {
    let hwnd = ensure_main_hwnd();
    if hwnd != 0 {
        unsafe {
            // SW_SHOW = 5, SW_RESTORE = 9
            let _ = ShowWindow(hwnd, 5);
            let _ = ShowWindow(hwnd, 9);
            let _ = SetForegroundWindow(hwnd);
        }
    }
    if let Some(ctx) = GLOBAL_EGUI_CTX.get() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }
}

#[cfg(windows)]
static GLOBAL_EGUI_CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

#[cfg(windows)]
fn create_studio_hicon() -> isize {
    let w = 16usize;
    let h = 16usize;
    let and_mask = vec![0u8; (w * h) / 8];
    let mut xor_bgra = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            // 绘制闪电形状
            let in_bolt = (y >= 2 && y <= 8 && x >= (9usize.saturating_sub(y)) && x <= 9)
                || (y >= 7 && y <= 13 && x >= 6 && x <= (16usize.saturating_sub(y)));
            if in_bolt {
                xor_bgra[idx] = 255; // B
                xor_bgra[idx + 1] = 255; // G
                xor_bgra[idx + 2] = 255; // R
                xor_bgra[idx + 3] = 255; // A
            } else {
                xor_bgra[idx] = 235; // B (靛蓝)
                xor_bgra[idx + 1] = 95; // G
                xor_bgra[idx + 2] = 82; // R
                xor_bgra[idx + 3] = 255; // A
            }
        }
    }
    unsafe { CreateIcon(0, 16, 16, 1, 32, and_mask.as_ptr(), xor_bgra.as_ptr()) }
}

#[cfg(windows)]
const WM_TRAYICON: u32 = 0x0400 + 88;

#[cfg(windows)]
unsafe extern "system" fn tray_wnd_proc(
    hwnd: isize,
    msg: u32,
    w_param: usize,
    l_param: isize,
) -> isize {
    if msg == WM_TRAYICON {
        let ev = (l_param as u32) & 0xFFFF;
        // WM_LBUTTONUP (0x0202) | WM_LBUTTONDBLCLK (0x0203)
        if ev == 0x0202 || ev == 0x0203 {
            restore_main_window_from_tray();
            return 0;
        }
        // WM_RBUTTONUP (0x0205) | WM_CONTEXTMENU (0x007B)
        if ev == 0x0205 || ev == 0x007B {
            let menu = CreatePopupMenu();
            if menu != 0 {
                let open_txt = to_wide("打开 Y3 Studio 主界面");
                let exit_txt = to_wide("彻底退出 Y3 Studio");
                // MF_STRING = 0x0000, MF_SEPARATOR = 0x0800
                let _ = AppendMenuW(menu, 0x0000, 1001, open_txt.as_ptr());
                let _ = AppendMenuW(menu, 0x0800, 0, std::ptr::null());
                let _ = AppendMenuW(menu, 0x0000, 1009, exit_txt.as_ptr());

                let mut pt = Point { x: 0, y: 0 };
                let _ = GetCursorPos(&mut pt);
                let _ = SetForegroundWindow(hwnd);
                // TPM_RETURNCMD (0x0100) | TPM_NONOTIFY (0x0080) | TPM_RIGHTBUTTON (0x0002)
                let cmd = TrackPopupMenu(menu, 0x0182, pt.x, pt.y, 0, hwnd, std::ptr::null());
                let _ = DestroyMenu(menu);

                if cmd == 1001 {
                    restore_main_window_from_tray();
                } else if cmd == 1009 {
                    let mut nid: NotifyIconDataW = std::mem::zeroed();
                    nid.cb_size = std::mem::size_of::<NotifyIconDataW>() as u32;
                    nid.h_wnd = hwnd;
                    nid.u_id = 1;
                    // NIM_DELETE = 2
                    let _ = Shell_NotifyIconW(2, &nid);
                    std::process::exit(0);
                }
            }
            return 0;
        }
    }
    DefWindowProcW(hwnd, msg, w_param, l_param)
}

#[cfg(windows)]
fn spawn_system_tray_thread(mcp_port: u16) {
    std::thread::spawn(move || unsafe {
        let class_name = to_wide("Y3StudioTrayClass");
        let h_inst = GetModuleHandleW(std::ptr::null());
        let wc = WndClassW {
            style: 0,
            lpfn_wnd_proc: tray_wnd_proc,
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance: h_inst,
            h_icon: 0,
            h_cursor: 0,
            hbr_background: 0,
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: class_name.as_ptr(),
        };
        let _ = RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            h_inst,
            std::ptr::null(),
        );
        if hwnd == 0 {
            return;
        }

        let mut nid: NotifyIconDataW = std::mem::zeroed();
        nid.cb_size = std::mem::size_of::<NotifyIconDataW>() as u32;
        nid.h_wnd = hwnd;
        nid.u_id = 1;
        // NIF_MESSAGE (0x01) | NIF_ICON (0x02) | NIF_TIP (0x04)
        nid.u_flags = 0x01 | 0x02 | 0x04;
        nid.u_callback_message = WM_TRAYICON;
        nid.h_icon = create_studio_hicon();

        let tip = to_wide(&format!(
            "Y3 Studio v{} (MCP :{}) — 左键打开 / 右键退出",
            env!("CARGO_PKG_VERSION"),
            mcp_port
        ));
        for (i, &ch) in tip.iter().take(127).enumerate() {
            nid.sz_tip[i] = ch;
        }

        // NIM_ADD = 0
        let _ = Shell_NotifyIconW(0, &nid);

        let mut msg: Msg = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    });
}

#[cfg(windows)]
fn trim_self_working_set() {
    unsafe {
        let _ = K32EmptyWorkingSet(GetCurrentProcess());
    }
}

pub async fn restore_saved_projects(dispatcher: &Arc<ToolDispatcher>) {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let cfg_path = PathBuf::from(appdata).join("y3-helper").join("projects.json");
        if let Ok(text) = std::fs::read_to_string(cfg_path) {
            let clean = text.trim_start_matches('\u{feff}');
            if let Ok(v) = serde_json::from_str::<Value>(clean) {
                if let Some(arr) = v.get("projects").and_then(|a| a.as_array()) {
                    for item in arr {
                        if let Some(p_str) = item.as_str() {
                            let p = PathBuf::from(p_str);
                            if p.exists() {
                                let _ = dispatcher.manager.register_project(&p, false).await;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn save_projects_to_disk(paths: &[String]) {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let dir = PathBuf::from(appdata).join("y3-helper");
        let _ = std::fs::create_dir_all(&dir);
        let cfg_path = dir.join("projects.json");
        let payload = json!({ "projects": paths });
        let _ = std::fs::write(
            cfg_path,
            serde_json::to_string_pretty(&payload).unwrap_or_default(),
        );
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Y3Maker,
    Console,
    Table,
    UiMeta,
    Build,
    MultiMcp,
}

#[allow(dead_code)]
#[derive(Clone, Default)]
struct ProjectItemUi {
    project_id: String,
    project_path: String,
    current_map_name: String,
    maps: Vec<String>,
    console_port: u16,
    client_count: usize,
}

#[allow(dead_code)]
enum AsyncMsg {
    StatusRefresh {
        projects: Vec<ProjectItemUi>,
        editor_exe: String,
        hub: Option<Y3MakerHubStatus>,
        force_active_id: Option<String>,
    },
    HubUpdated {
        hub: Y3MakerHubStatus,
        toast: String,
    },
    SkillDetailLoaded {
        title: String,
        content: String,
    },
    LogsLoaded(String),
    TableListLoaded(Vec<(u64, String)>),
    TableDetailLoaded {
        key: String,
        name: String,
        json_text: String,
    },
    UiTreeLoaded(String),
    MetaOutputLoaded(String),
    BuildOutputLoaded(String),
    EditorMcpsLoaded(Vec<Value>),
    Toast {
        msg: String,
        is_err: bool,
    },
    FolderPicked(String),
    EditorExePicked(String),
}

pub struct Y3StudioApp {
    dispatcher: Arc<ToolDispatcher>,
    rt: tokio::runtime::Handle,
    egui_ctx: egui::Context,
    mcp_port: u16,
    tx: mpsc::Sender<AsyncMsg>,
    rx: mpsc::Receiver<AsyncMsg>,

    projects: Vec<ProjectItemUi>,
    active_project_id: Option<String>,
    active_map_name: Option<String>,
    editor_exe: String,
    active_tab: ActiveTab,

    status_msg: String,
    status_is_err: bool,

    // Tab 1: Y3Maker & Agent Hub
    hub_status: Option<Y3MakerHubStatus>,
    sync_source: String,
    custom_git_url: String,
    selected_agent_targets: Vec<(String, bool)>,
    skill_modal: Option<(String, String)>,

    // Tab 2: Console & Lua
    lua_code: String,
    game_logs: String,
    ui_touch_path: String,

    // Tab 3: Editor Table
    table_type: String,
    table_search: String,
    table_items: Vec<(u64, String)>,
    edit_obj_key: String,
    edit_obj_name: String,
    edit_obj_json: String,

    // Tab 4: UI Tree & Meta
    ui_node_path: String,
    ui_depth: i32,
    ui_tree_output: String,
    meta_output: String,

    // Tab 5: ECA & GMP Build
    build_output: String,
    gmp_prefabs: bool,
    gmp_ui: bool,
    gmp_zh: bool,
    gmp_us: bool,

    // Tab 6: Multi-Project & Editor MCPs
    running_editors: Vec<Value>,
}

impl Y3StudioApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        dispatcher: Arc<ToolDispatcher>,
        mcp_port: u16,
    ) -> Self {
        setup_chinese_fonts(&cc.egui_ctx);
        setup_dark_theme(&cc.egui_ctx);

        let (tx, rx) = mpsc::channel();
        let rt = tokio::runtime::Handle::current();

        let app = Self {
            dispatcher,
            rt,
            egui_ctx: cc.egui_ctx.clone(),
            mcp_port,
            tx,
            rx,
            projects: Vec::new(),
            active_project_id: None,
            active_map_name: None,
            editor_exe: String::new(),
            active_tab: ActiveTab::Y3Maker,
            status_msg: "就绪".to_string(),
            status_is_err: false,
            hub_status: None,
            sync_source: "local".to_string(),
            custom_git_url: String::new(),
            selected_agent_targets: vec![
                ("project_links".to_string(), true),
                ("claude".to_string(), true),
                ("cursor".to_string(), true),
                ("windsurf".to_string(), true),
                ("codex".to_string(), true),
            ],
            skill_modal: None,
            lua_code: "print('Hello Y3 Studio!')".to_string(),
            game_logs: String::new(),
            ui_touch_path: "panel_1.button_1".to_string(),
            table_type: "unit".to_string(),
            table_search: String::new(),
            table_items: Vec::new(),
            edit_obj_key: String::new(),
            edit_obj_name: String::new(),
            edit_obj_json: "{\n  \n}".to_string(),
            ui_node_path: String::new(),
            ui_depth: 2,
            ui_tree_output: String::new(),
            meta_output: String::new(),
            build_output: String::new(),
            gmp_prefabs: true,
            gmp_ui: true,
            gmp_zh: true,
            gmp_us: false,
            running_editors: Vec::new(),
        };

        app.trigger_refresh();
        #[cfg(windows)]
        {
            let _ = GLOBAL_EGUI_CTX.set(cc.egui_ctx.clone());
            spawn_system_tray_thread(mcp_port);
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(400));
                let _ = ensure_main_hwnd();
                std::thread::sleep(std::time::Duration::from_millis(800));
                trim_self_working_set();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    trim_self_working_set();
                }
            });
        }
        app
    }

    fn set_toast(&mut self, msg: impl Into<String>, is_err: bool) {
        self.status_msg = msg.into();
        self.status_is_err = is_err;
    }

    fn active_project(&self) -> Option<&ProjectItemUi> {
        let id = self.active_project_id.as_ref()?;
        self.projects.iter().find(|p| &p.project_id == id)
    }

    async fn fetch_and_send_refresh(
        disp: &Arc<ToolDispatcher>,
        tx: &mpsc::Sender<AsyncMsg>,
        egui_ctx: &egui::Context,
        active_id: Option<&str>,
        mcp_port: u16,
    ) {
        let status = disp.manager.list_projects_json();
        let editor_exe = status
            .get("editor")
            .and_then(|e| e.get("editor_exe_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("未配置")
            .to_string();

        let mut projects = Vec::new();
        if let Some(arr) = status.get("projects").and_then(|v| v.as_array()) {
            for p in arr {
                let project_id = p
                    .get("project_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let project_path = p
                    .get("project_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let current_map_name = p
                    .get("current_map_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("EntryMap")
                    .to_string();
                let console_port = p
                    .get("console_port")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u16;
                let client_count = p
                    .get("connected_clients")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let maps = p
                    .get("maps")
                    .and_then(|v| v.as_array())
                    .map(|m_arr| {
                        m_arr
                            .iter()
                            .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_else(|| vec![current_map_name.clone()]);

                projects.push(ProjectItemUi {
                    project_id,
                    project_path,
                    current_map_name,
                    maps,
                    console_port,
                    client_count,
                });
            }
        }

        let target_sel = active_id
            .filter(|id| projects.iter().any(|p| p.project_id == *id))
            .or_else(|| projects.first().map(|p| p.project_id.as_str()));
        let hub = if let Some(sel) = target_sel {
            if let Ok(ws) = disp.manager.resolve_project(Some(sel)).await {
                let ctx = ws.context.read().clone();
                Some(ProjectInitializer::inspect_hub_status(&ctx, mcp_port))
            } else {
                None
            }
        } else {
            None
        };

        let _ = tx.send(AsyncMsg::StatusRefresh {
            projects,
            editor_exe,
            hub,
            force_active_id: active_id.map(|s| s.to_string()),
        });
        egui_ctx.request_repaint();
    }

    fn trigger_refresh(&self) {
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        let active_id = self.active_project_id.clone();
        let mcp_port = self.mcp_port;

        self.rt.spawn(async move {
            Self::fetch_and_send_refresh(&disp, &tx, &egui_ctx, active_id.as_deref(), mcp_port)
                .await;
        });
    }

    fn call_tool_async(&self, tool: &'static str, mut args: Value, label: &'static str) {
        if let Some(obj) = args.as_object_mut() {
            if !obj.contains_key("projectPath") {
                if let Some(pid) = &self.active_project_id {
                    obj.insert("projectPath".to_string(), json!(pid));
                }
            }
            if !obj.contains_key("mapName") {
                if let Some(m) = &self.active_map_name {
                    obj.insert("mapName".to_string(), json!(m));
                }
            }
        }
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        self.rt.spawn(async move {
            match disp.call_tool(tool, &args).await {
                Ok(res) => {
                    let pretty = serde_json::to_string_pretty(&res).unwrap_or_default();
                    let _ = tx.send(AsyncMsg::BuildOutputLoaded(pretty));
                    let msg = res
                        .get("message")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("{} 执行完成", label));
                    let _ = tx.send(AsyncMsg::Toast {
                        msg,
                        is_err: false,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AsyncMsg::BuildOutputLoaded(format!("{} 失败:\n{}", label, e)));
                    let _ = tx.send(AsyncMsg::Toast {
                        msg: format!("{} 失败: {}", label, e),
                        is_err: true,
                    });
                }
            }
            egui_ctx.request_repaint();
        });
    }

    fn init_current_project(&mut self) {
        let Some(pid) = self.active_project_id.clone() else {
            return;
        };
        let map = self.active_map_name.clone();
        let repo_src = if self.sync_source == "github" {
            "github".to_string()
        } else {
            "gitee".to_string()
        };
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        let mcp_port = self.mcp_port;
        self.set_toast(
            "正在初始化项目 (克隆 y3-lualib 到 script/y3、复制项目配置并初始化 .y3maker)...",
            false,
        );

        self.rt.spawn(async move {
            match disp
                .call_tool(
                    "init_project",
                    &json!({
                        "projectPath": pid,
                        "mapName": map,
                        "repoSource": repo_src
                    }),
                )
                .await
            {
                Ok(res) => {
                    let pretty = serde_json::to_string_pretty(&res).unwrap_or_default();
                    let _ = tx.send(AsyncMsg::BuildOutputLoaded(pretty));
                    let _ = disp
                        .call_tool(
                            "sync_y3maker",
                            &json!({
                                "projectPath": pid,
                                "repoSource": "local",
                                "mcpPort": mcp_port
                            }),
                        )
                        .await;
                    let map_str = res
                        .get("map_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("EntryMap");
                    let _ = tx.send(AsyncMsg::Toast {
                        msg: format!("项目初始化完成（已就绪 {}/script/y3 与 .y3maker）", map_str),
                        is_err: false,
                    });
                    Self::fetch_and_send_refresh(&disp, &tx, &egui_ctx, Some(&pid), mcp_port).await;
                }
                Err(e) => {
                    let _ = tx.send(AsyncMsg::BuildOutputLoaded(format!(
                        "初始化项目失败:\n{}",
                        e
                    )));
                    let _ = tx.send(AsyncMsg::Toast {
                        msg: format!("初始化项目失败: {}", e),
                        is_err: true,
                    });
                    egui_ctx.request_repaint();
                }
            }
        });
    }

    fn sync_y3maker(&mut self) {
        let Some(pid) = self.active_project_id.clone() else {
            return;
        };
        let src = if self.sync_source == "custom" {
            self.custom_git_url.trim().to_string()
        } else {
            self.sync_source.clone()
        };
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        let mcp_port = self.mcp_port;
        self.set_toast("正在同步 .y3maker 技能库...", false);

        self.rt.spawn(async move {
            match disp
                .call_tool(
                    "sync_y3maker",
                    &json!({
                        "projectPath": pid,
                        "repoSource": src,
                        "mcpPort": mcp_port
                    }),
                )
                .await
            {
                Ok(res) => {
                    if let Some(hub_val) = res.get("hub") {
                        if let Ok(hub) = serde_json::from_value::<Y3MakerHubStatus>(hub_val.clone())
                        {
                            let _ = tx.send(AsyncMsg::HubUpdated {
                                toast: format!("技能库同步完成（共 {} 个技能）", hub.skills_count),
                                hub,
                            });
                            egui_ctx.request_repaint();
                            return;
                        }
                    }
                    let _ = tx.send(AsyncMsg::Toast {
                        msg: "同步完成".to_string(),
                        is_err: false,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AsyncMsg::Toast {
                        msg: format!("同步失败: {}", e),
                        is_err: true,
                    });
                }
            }
            egui_ctx.request_repaint();
        });
    }

    fn equip_selected_agents(&mut self) {
        let Some(pid) = self.active_project_id.clone() else {
            return;
        };
        let targets: Vec<String> = self
            .selected_agent_targets
            .iter()
            .filter(|(_, checked)| *checked)
            .map(|(id, _)| id.clone())
            .collect();
        if targets.is_empty() {
            self.set_toast("请至少勾选一个智能体", true);
            return;
        }
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        let mcp_port = self.mcp_port;
        self.set_toast("正在装配选中智能体...", false);

        self.rt.spawn(async move {
            match disp
                .call_tool(
                    "equip_agents",
                    &json!({
                        "projectPath": pid,
                        "targets": targets,
                        "mcpPort": mcp_port
                    }),
                )
                .await
            {
                Ok(res) => {
                    if let Some(hub_val) = res.get("hub") {
                        if let Ok(hub) = serde_json::from_value::<Y3MakerHubStatus>(hub_val.clone())
                        {
                            let equipped_cnt =
                                hub.agent_targets.iter().filter(|t| t.equipped).count();
                            let _ = tx.send(AsyncMsg::HubUpdated {
                                toast: format!(
                                    "装配成功（已就绪 {} 个智能体，{} 个技能）",
                                    equipped_cnt, hub.skills_count
                                ),
                                hub,
                            });
                            egui_ctx.request_repaint();
                            return;
                        }
                    }
                    let _ = tx.send(AsyncMsg::Toast {
                        msg: "装配完成".to_string(),
                        is_err: false,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AsyncMsg::Toast {
                        msg: format!("装配失败: {}", e),
                        is_err: true,
                    });
                }
            }
            egui_ctx.request_repaint();
        });
    }

    fn load_skill_detail(&self, skill_id: String) {
        let Some(pid) = self.active_project_id.clone() else {
            return;
        };
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        self.rt.spawn(async move {
            if let Ok(res) = disp
                .call_tool(
                    "get_skill_detail",
                    &json!({ "projectPath": pid, "skillId": skill_id }),
                )
                .await
            {
                let content = res
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let _ = tx.send(AsyncMsg::SkillDetailLoaded {
                    title: skill_id,
                    content,
                });
                egui_ctx.request_repaint();
            }
        });
    }

    fn load_logs(&self) {
        let Some(pid) = self.active_project_id.clone() else {
            return;
        };
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        self.rt.spawn(async move {
            if let Ok(res) = disp
                .call_tool("get_logs", &json!({ "projectPath": pid, "limit": 120 }))
                .await
            {
                let text = if let Some(arr) = res.get("logs").and_then(|v| v.as_array()) {
                    arr.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    serde_json::to_string_pretty(&res).unwrap_or_default()
                };
                let _ = tx.send(AsyncMsg::LogsLoaded(text));
                egui_ctx.request_repaint();
            }
        });
    }

    fn load_table_list(&self) {
        let Some(pid) = self.active_project_id.clone() else {
            return;
        };
        let map = self.active_map_name.clone();
        let tt = self.table_type.clone();
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        self.rt.spawn(async move {
            if let Ok(res) = disp
                .call_tool(
                    "editor_table_list",
                    &json!({ "projectPath": pid, "mapName": map, "tableType": tt }),
                )
                .await
            {
                let mut list = Vec::new();
                if let Some(arr) = res.get("items").and_then(|v| v.as_array()) {
                    for it in arr {
                        let k = it.get("key").and_then(|v| v.as_u64()).unwrap_or(0);
                        let n = it
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        list.push((k, n));
                    }
                }
                let _ = tx.send(AsyncMsg::TableListLoaded(list));
                egui_ctx.request_repaint();
            }
        });
    }

    fn load_table_item(&self, key: u64) {
        let Some(pid) = self.active_project_id.clone() else {
            return;
        };
        let map = self.active_map_name.clone();
        let tt = self.table_type.clone();
        let disp = self.dispatcher.clone();
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        self.rt.spawn(async move {
            if let Ok(res) = disp
                .call_tool(
                    "editor_table_get",
                    &json!({ "projectPath": pid, "mapName": map, "tableType": tt, "key": key }),
                )
                .await
            {
                let name = res
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let data = res.get("data").cloned().unwrap_or_else(|| json!({}));
                let json_text = serde_json::to_string_pretty(&data).unwrap_or_default();
                let _ = tx.send(AsyncMsg::TableDetailLoaded {
                    key: key.to_string(),
                    name,
                    json_text,
                });
                egui_ctx.request_repaint();
            }
        });
    }
}

impl eframe::App for Y3StudioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            #[cfg(windows)]
            hide_main_window_to_tray();
            return;
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                AsyncMsg::StatusRefresh {
                    projects,
                    editor_exe,
                    hub,
                    force_active_id,
                } => {
                    self.projects = projects;
                    self.editor_exe = editor_exe;
                    self.hub_status = hub;
                    if let Some(fid) = force_active_id {
                        if let Some(found) = self.projects.iter().find(|p| p.project_id == fid) {
                            self.active_project_id = Some(found.project_id.clone());
                            self.active_map_name = Some(found.current_map_name.clone());
                        }
                    }
                    if self.active_project_id.is_none()
                        || !self
                            .projects
                            .iter()
                            .any(|p| Some(&p.project_id) == self.active_project_id.as_ref())
                    {
                        if let Some(first) = self.projects.first() {
                            self.active_project_id = Some(first.project_id.clone());
                            self.active_map_name = Some(first.current_map_name.clone());
                        } else {
                            self.active_project_id = None;
                            self.active_map_name = None;
                        }
                    }
                    let saved_paths: Vec<String> =
                        self.projects.iter().map(|p| p.project_path.clone()).collect();
                    save_projects_to_disk(&saved_paths);
                }
                AsyncMsg::HubUpdated { hub, toast } => {
                    self.hub_status = Some(hub);
                    self.set_toast(toast, false);
                }
                AsyncMsg::SkillDetailLoaded { title, content } => {
                    self.skill_modal = Some((title, content));
                }
                AsyncMsg::LogsLoaded(t) => self.game_logs = t,
                AsyncMsg::TableListLoaded(items) => self.table_items = items,
                AsyncMsg::TableDetailLoaded {
                    key,
                    name,
                    json_text,
                } => {
                    self.edit_obj_key = key;
                    self.edit_obj_name = name;
                    self.edit_obj_json = json_text;
                }
                AsyncMsg::UiTreeLoaded(t) => self.ui_tree_output = t,
                AsyncMsg::MetaOutputLoaded(t) => self.meta_output = t,
                AsyncMsg::BuildOutputLoaded(t) => self.build_output = t,
                AsyncMsg::EditorMcpsLoaded(list) => self.running_editors = list,
                AsyncMsg::Toast { msg, is_err } => self.set_toast(msg, is_err),
                AsyncMsg::FolderPicked(path) => {
                    let disp = self.dispatcher.clone();
                    let tx = self.tx.clone();
                    let egui_ctx = self.egui_ctx.clone();
                    let mcp_port = self.mcp_port;
                    self.rt.spawn(async move {
                        match disp
                            .call_tool(
                                "set_project_path",
                                &json!({ "projectPath": path, "setActive": true }),
                            )
                            .await
                        {
                            Ok(res) => {
                                let new_id = res
                                    .get("status")
                                    .and_then(|s| s.get("project_id"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                let _ = tx.send(AsyncMsg::Toast {
                                    msg: format!("已添加项目: {}", path),
                                    is_err: false,
                                });
                                Self::fetch_and_send_refresh(
                                    &disp,
                                    &tx,
                                    &egui_ctx,
                                    new_id.as_deref(),
                                    mcp_port,
                                )
                                .await;
                            }
                            Err(e) => {
                                let _ = tx.send(AsyncMsg::Toast {
                                    msg: format!("添加项目失败: {}", e),
                                    is_err: true,
                                });
                                egui_ctx.request_repaint();
                            }
                        }
                    });
                }
                AsyncMsg::EditorExePicked(path) => {
                    let disp = self.dispatcher.clone();
                    let tx = self.tx.clone();
                    let egui_ctx = self.egui_ctx.clone();
                    let active_id = self.active_project_id.clone();
                    let mcp_port = self.mcp_port;
                    self.rt.spawn(async move {
                        let _ = disp
                            .call_tool("set_editor_path", &json!({ "editorPath": path }))
                            .await;
                        let _ = tx.send(AsyncMsg::Toast {
                            msg: "已更新编辑器路径".to_string(),
                            is_err: false,
                        });
                        Self::fetch_and_send_refresh(
                            &disp,
                            &tx,
                            &egui_ctx,
                            active_id.as_deref(),
                            mcp_port,
                        )
                        .await;
                    });
                }
            }
        }

        // 底部精简状态栏
        egui::TopBottomPanel::bottom("bottom_status")
            .exact_height(26.0)
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(10, 12, 17))
                    .inner_margin(Margin::symmetric(12.0, 4.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(28, 33, 46))),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let color = if self.status_is_err {
                        Color32::from_rgb(248, 113, 113)
                    } else {
                        Color32::from_rgb(147, 197, 253)
                    };
                    ui.colored_label(color, &self.status_msg);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "v{}  |  MCP :{}",
                                env!("CARGO_PKG_VERSION"),
                                self.mcp_port
                            ))
                            .size(11.5)
                            .color(Color32::from_rgb(110, 118, 138)),
                        );
                    });
                });
            });

        // 最左栏：项目列表 (230px)
        egui::SidePanel::left("left_projects_rail")
            .resizable(false)
            .exact_width(235.0)
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(13, 16, 23))
                    .inner_margin(Margin::same(10.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(28, 33, 46))),
            )
            .show(ctx, |ui| {
                self.render_left_projects_panel(ui);
            });

        // 次左栏：当前项目功能导航 (155px)
        egui::SidePanel::left("sub_nav_rail")
            .resizable(false)
            .exact_width(155.0)
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(16, 20, 29))
                    .inner_margin(Margin::same(10.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(28, 33, 46))),
            )
            .show(ctx, |ui| {
                self.render_sub_nav_panel(ui);
            });

        // 顶部快捷操作栏
        egui::TopBottomPanel::top("top_action_bar")
            .exact_height(46.0)
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(15, 18, 26))
                    .inner_margin(Margin::symmetric(14.0, 8.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(28, 33, 46))),
            )
            .show(ctx, |ui| {
                self.render_top_action_bar(ui);
            });

        // 右侧主内容区
        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(11, 13, 19))
                    .inner_margin(Margin::same(14.0)),
            )
            .show(ctx, |ui| {
                if self.active_project_id.is_none() && self.active_tab != ActiveTab::MultiMcp {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("请在左侧添加或选择一个 Y3 项目")
                                .size(15.0)
                                .color(Color32::from_rgb(120, 130, 150)),
                        );
                    });
                    return;
                }
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    match self.active_tab {
                        ActiveTab::Y3Maker => self.render_tab_y3maker(ui),
                        ActiveTab::Console => self.render_tab_console(ui),
                        ActiveTab::Table => self.render_tab_table(ui),
                        ActiveTab::UiMeta => self.render_tab_ui_meta(ui),
                        ActiveTab::Build => self.render_tab_build(ui),
                        ActiveTab::MultiMcp => self.render_tab_multi_mcp(ui),
                    }
                });
            });

        // 技能详情弹窗
        if let Some((title, content)) = self.skill_modal.clone() {
            let mut open = true;
            egui::Window::new(format!("技能: {}", title))
                .open(&mut open)
                .default_size([680.0, 520.0])
                .resizable(true)
                .show(ctx, |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut content.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    });
                });
            if !open {
                self.skill_modal = None;
            }
        }
    }
}

impl Y3StudioApp {
    fn render_left_projects_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("项目列表")
                    .strong()
                    .size(13.5)
                    .color(Color32::WHITE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("+ 添加项目").size(12.0).color(Color32::WHITE))
                            .fill(Color32::from_rgb(79, 70, 229)),
                    )
                    .clicked()
                {
                    let tx = self.tx.clone();
                    let egui_ctx = self.egui_ctx.clone();
                    std::thread::spawn(move || {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("选择 Y3 项目根目录")
                            .pick_folder()
                        {
                            let _ = tx.send(AsyncMsg::FolderPicked(
                                p.to_string_lossy().to_string(),
                            ));
                            egui_ctx.request_repaint();
                        }
                    });
                }
            });
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        let mut select_proj: Option<(String, String)> = None;
        let mut remove_proj: Option<String> = None;
        let mut switch_map: Option<(String, String)> = None;

        ScrollArea::vertical()
            .max_height(ui.available_height() - 95.0)
            .show(ui, |ui| {
                for p in &self.projects {
                    let is_active = Some(&p.project_id) == self.active_project_id.as_ref();
                    let bg = if is_active {
                        Color32::from_rgb(28, 36, 56)
                    } else {
                        Color32::from_rgb(19, 23, 34)
                    };
                    let border = if is_active {
                        Color32::from_rgb(99, 102, 241)
                    } else {
                        Color32::from_rgb(34, 40, 56)
                    };

                    Frame::none()
                        .fill(bg)
                        .stroke(Stroke::new(1.0_f32, border))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let name = p
                                    .project_path
                                    .split(['\\', '/'])
                                    .filter(|s| !s.is_empty())
                                    .next_back()
                                    .unwrap_or(&p.project_id);
                                if ui
                                    .selectable_label(
                                        is_active,
                                        RichText::new(name).strong().size(13.0),
                                    )
                                    .clicked()
                                {
                                    select_proj =
                                        Some((p.project_id.clone(), p.current_map_name.clone()));
                                }
                                ui.label(
                                    RichText::new(format!(":{}", p.console_port))
                                        .size(11.0)
                                        .color(Color32::from_rgb(129, 140, 248)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button(
                                                RichText::new("×")
                                                    .color(Color32::from_rgb(160, 170, 190)),
                                            )
                                            .on_hover_text("移除项目")
                                            .clicked()
                                        {
                                            remove_proj = Some(p.project_id.clone());
                                        }
                                    },
                                );
                            });

                            ui.label(
                                RichText::new(&p.project_path)
                                    .size(10.5)
                                    .color(Color32::from_rgb(115, 125, 145)),
                            );

                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                for m in &p.maps {
                                    let m_selected = is_active
                                        && self.active_map_name.as_deref() == Some(m.as_str());
                                    if ui
                                        .selectable_label(m_selected, RichText::new(m).size(11.0))
                                        .clicked()
                                    {
                                        switch_map = Some((p.project_id.clone(), m.clone()));
                                    }
                                }
                            });
                        });
                    ui.add_space(6.0);
                }
            });

        if let Some((pid, map)) = select_proj {
            self.active_project_id = Some(pid.clone());
            self.active_map_name = Some(map);
            let disp = self.dispatcher.clone();
            let tx = self.tx.clone();
            let egui_ctx = self.egui_ctx.clone();
            let mcp_port = self.mcp_port;
            self.rt.spawn(async move {
                let _ = disp
                    .call_tool(
                        "set_project_path",
                        &json!({ "projectPath": pid, "setActive": true }),
                    )
                    .await;
                Self::fetch_and_send_refresh(&disp, &tx, &egui_ctx, Some(&pid), mcp_port).await;
            });
        }
        if let Some((pid, map)) = switch_map {
            self.active_project_id = Some(pid.clone());
            self.active_map_name = Some(map.clone());
            let disp = self.dispatcher.clone();
            let tx = self.tx.clone();
            let egui_ctx = self.egui_ctx.clone();
            let mcp_port = self.mcp_port;
            self.rt.spawn(async move {
                let _ = disp
                    .call_tool(
                        "switch_map",
                        &json!({ "projectPath": pid, "mapName": map }),
                    )
                    .await;
                let _ = tx.send(AsyncMsg::Toast {
                    msg: format!("已切换子地图: {}", map),
                    is_err: false,
                });
                Self::fetch_and_send_refresh(&disp, &tx, &egui_ctx, Some(&pid), mcp_port).await;
            });
        }
        if let Some(pid) = remove_proj {
            let next_active = self
                .projects
                .iter()
                .find(|p| p.project_id != pid)
                .map(|p| p.project_id.clone());
            self.active_project_id = next_active.clone();
            let disp = self.dispatcher.clone();
            let tx = self.tx.clone();
            let egui_ctx = self.egui_ctx.clone();
            let mcp_port = self.mcp_port;
            self.rt.spawn(async move {
                let _ = disp
                    .call_tool("remove_project", &json!({ "projectPath": pid }))
                    .await;
                let _ = tx.send(AsyncMsg::Toast {
                    msg: "已移除项目".to_string(),
                    is_err: false,
                });
                Self::fetch_and_send_refresh(
                    &disp,
                    &tx,
                    &egui_ctx,
                    next_active.as_deref(),
                    mcp_port,
                )
                .await;
            });
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            if ui
                .add_sized(
                    [ui.available_width(), 28.0],
                    egui::Button::new(
                        RichText::new("复制 MCP 配置")
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(79, 70, 229)),
                )
                .clicked()
            {
                let base = format!("http://127.0.0.1:{}", self.mcp_port);
                let cfg = serde_json::to_string_pretty(&json!({
                    "mcpServers": {
                        "y3-helper": { "url": format!("{}/mcp", base) },
                        "y3editor": { "url": format!("{}/y3editor/mcp", base) },
                        "y3runtime": { "url": format!("{}/y3runtime/mcp", base) }
                    }
                }))
                .unwrap_or_default();
                ui.ctx().copy_text(cfg);
                self.set_toast("已复制 MCP 配置到剪贴板", false);
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("编辑器")
                        .size(11.5)
                        .color(Color32::from_rgb(150, 160, 180)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("更改").clicked() {
                        let tx = self.tx.clone();
                        std::thread::spawn(move || {
                            if let Some(f) = rfd::FileDialog::new()
                                .add_filter("Editor", &["exe"])
                                .pick_file()
                            {
                                let _ = tx.send(AsyncMsg::EditorExePicked(
                                    f.to_string_lossy().to_string(),
                                ));
                            }
                        });
                    }
                });
            });
        });
    }

    fn render_sub_nav_panel(&mut self, ui: &mut egui::Ui) {
        let proj_title = self
            .active_project()
            .map(|p| {
                p.project_path
                    .split(['\\', '/'])
                    .filter(|s| !s.is_empty())
                    .next_back()
                    .unwrap_or(&p.project_id)
                    .to_string()
            })
            .unwrap_or_else(|| "未选择项目".to_string());

        ui.label(
            RichText::new(proj_title)
                .strong()
                .size(13.5)
                .color(Color32::from_rgb(165, 180, 252)),
        );
        if let Some(m) = &self.active_map_name {
            ui.label(
                RichText::new(m)
                    .size(11.5)
                    .color(Color32::from_rgb(110, 231, 183)),
            );
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        let nav_items = [
            (ActiveTab::Y3Maker, "技能库 (.y3maker)"),
            (ActiveTab::Console, "调试与日志"),
            (ActiveTab::Table, "物编与导表"),
            (ActiveTab::Build, "编译与打包"),
            (ActiveTab::MultiMcp, "多项目与网关"),
        ];

        for (tab, label) in nav_items {
            let selected = self.active_tab == tab;
            let resp = ui.add_sized(
                [ui.available_width(), 30.0],
                egui::SelectableLabel::new(
                    selected,
                    RichText::new(label).size(12.5).color(if selected {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(170, 178, 198)
                    }),
                ),
            );
            if resp.clicked() {
                self.active_tab = tab;
                match tab {
                    ActiveTab::Y3Maker => self.trigger_refresh(),
                    ActiveTab::Console => self.load_logs(),
                    ActiveTab::Table => self.load_table_list(),
                    ActiveTab::MultiMcp => {
                        let disp = self.dispatcher.clone();
                        let tx = self.tx.clone();
                        self.rt.spawn(async move {
                            if let Ok(res) = disp.call_tool("discover_editor_mcps", &json!({})).await
                            {
                                let list = res
                                    .get("editors")
                                    .and_then(|v| v.as_array())
                                    .cloned()
                                    .unwrap_or_default();
                                let _ = tx.send(AsyncMsg::EditorMcpsLoaded(list));
                            }
                        });
                    }
                    _ => {}
                }
            }
            ui.add_space(3.0);
        }
    }

    fn render_top_action_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let enabled = self.active_project_id.is_some();
            ui.add_enabled_ui(enabled, |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("▶ 启动游戏").color(Color32::WHITE))
                            .fill(Color32::from_rgb(16, 185, 129)),
                    )
                    .clicked()
                {
                    self.call_tool_async("launch_game", json!({}), "启动游戏");
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("打开编辑器").color(Color32::WHITE))
                            .fill(Color32::from_rgb(79, 70, 229)),
                    )
                    .clicked()
                {
                    self.call_tool_async("launch_editor", json!({}), "打开编辑器");
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("初始化项目").color(Color32::WHITE))
                            .fill(Color32::from_rgb(37, 99, 235)),
                    )
                    .on_hover_text(
                        "初始化新地图：克隆 y3-lualib 到 script/y3、复制默认项目配置并同步 .y3maker",
                    )
                    .clicked()
                {
                    self.init_current_project();
                }
                ui.separator();
                if ui.button("热重载 (.rd)").clicked() {
                    self.call_tool_async("reload_lua", json!({}), "热重载");
                }
                if ui.button("物编热更").clicked() {
                    self.call_tool_async("reload_editor_table", json!({}), "物编热更");
                }
                if ui.button("触发器热更").clicked() {
                    self.call_tool_async("reload_trigger", json!({}), "触发器热更");
                }
                if ui.button("重启 (.rr)").clicked() {
                    self.call_tool_async("quick_restart", json!({}), "重启游戏");
                }
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("结束游戏").color(Color32::from_rgb(252, 165, 165)),
                        )
                        .fill(Color32::from_rgb(69, 26, 32)),
                    )
                    .clicked()
                {
                    self.call_tool_async("kill_game", json!({ "killAll": false }), "结束游戏");
                }
                if ui.button("打开目录").clicked() {
                    self.call_tool_async("open_in_explorer", json!({}), "打开目录");
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("刷新").clicked() {
                    self.trigger_refresh();
                }
            });
        });
    }

    fn render_tab_y3maker(&mut self, ui: &mut egui::Ui) {
        let hub = self.hub_status.clone();

        // 区块 1：技能库同步 + 智能体装配（左右并排，简洁直观）
        ui.columns(2, |cols| {
            // 左卡：技能仓库同步
            card_frame().show(&mut cols[0], |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("技能库 (.y3maker)")
                            .strong()
                            .size(13.5)
                            .color(Color32::WHITE),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ready = hub.as_ref().map(|h| h.skills_count > 0).unwrap_or(false);
                        status_pill(
                            ui,
                            if ready { "已就绪" } else { "未同步" },
                            ready,
                        );
                    });
                });
                ui.add_space(8.0);

                if let Some(h) = &hub {
                    ui.horizontal_wrapped(|ui| {
                        metric_chip(ui, "技能", &format!("{}", h.skills_count));
                        metric_chip(ui, "知识库", &format!("{}", h.knowledge_count));
                        metric_chip(
                            ui,
                            "模板/工具",
                            &format!("{}", h.templates_count + h.tools_count),
                        );
                        metric_chip(
                            ui,
                            ".agents/skills",
                            if h.agents_skills_linked {
                                "已链接"
                            } else {
                                "未链接"
                            },
                        );
                        metric_chip(
                            ui,
                            "MCP 桥接",
                            if h.mcp_adapted { "已适配" } else { "待适配" },
                        );
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(&h.y3maker_dir)
                            .size(11.0)
                            .color(Color32::from_rgb(120, 130, 150)),
                    );
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("sync_source_combo")
                        .selected_text(match self.sync_source.as_str() {
                            "local" => "全局母库缓存 (极速)",
                            "github" => "GitHub 官方源",
                            "custom" => "自定义 Git URL",
                            _ => "全局母库缓存 (极速)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.sync_source,
                                "local".to_string(),
                                "全局母库缓存 (极速)",
                            );
                            ui.selectable_value(
                                &mut self.sync_source,
                                "github".to_string(),
                                "GitHub 官方源",
                            );
                            ui.selectable_value(
                                &mut self.sync_source,
                                "custom".to_string(),
                                "自定义 Git URL",
                            );
                        });

                    if ui
                        .add(
                            egui::Button::new(RichText::new("同步技能库").color(Color32::WHITE))
                                .fill(Color32::from_rgb(79, 70, 229)),
                        )
                        .clicked()
                    {
                        self.sync_y3maker();
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("初始化项目 (y3-lualib)").color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(37, 99, 235)),
                        )
                        .on_hover_text(
                            "新地图一键初始化：克隆 y3-lualib 到 script/y3、复制项目配置并同步 .y3maker",
                        )
                        .clicked()
                    {
                        self.init_current_project();
                    }
                });

                if self.sync_source == "custom" {
                    ui.add_space(6.0);
                    ui.add(
                        TextEdit::singleline(&mut self.custom_git_url)
                            .hint_text("https://.../y3-maker-config.git")
                            .desired_width(f32::INFINITY),
                    );
                }
            });

            // 右卡：AI 智能体装配
            card_frame().show(&mut cols[1], |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("AI 智能体装配")
                            .strong()
                            .size(13.5)
                            .color(Color32::WHITE),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("一键装配").strong().color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgb(16, 185, 129)),
                            )
                            .clicked()
                        {
                            self.equip_selected_agents();
                        }
                    });
                });
                ui.add_space(8.0);

                if let Some(h) = &hub {
                    for target in &h.agent_targets {
                        let chk_entry = self
                            .selected_agent_targets
                            .iter_mut()
                            .find(|(id, _)| id == &target.id);
                        ui.horizontal(|ui| {
                            if let Some((_, checked)) = chk_entry {
                                ui.checkbox(checked, "");
                            }
                            ui.label(
                                RichText::new(&target.name)
                                    .size(12.2)
                                    .color(Color32::from_rgb(225, 230, 242)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    status_pill(
                                        ui,
                                        if target.equipped {
                                            "已装配"
                                        } else {
                                            "未装配"
                                        },
                                        target.equipped,
                                    );
                                },
                            );
                        });
                        ui.add_space(3.0);
                    }
                }
            });
        });

        ui.add_space(12.0);

        // 区块 2：已安装技能网格（简洁卡片）
        card_frame().show(ui, |ui| {
            let skills = hub.as_ref().map(|h| h.skills.as_slice()).unwrap_or(&[]);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("已安装技能 ({})", skills.len()))
                        .strong()
                        .size(13.5)
                        .color(Color32::WHITE),
                );
            });
            ui.add_space(8.0);

            if skills.is_empty() {
                ui.label(
                    RichText::new("暂无技能，请点击上方「同步技能库」或「一键装配」")
                        .color(Color32::from_rgb(130, 140, 160)),
                );
            } else {
                let mut clicked_skill = None;
                egui::Grid::new("skills_grid")
                    .num_columns(3)
                    .spacing([10.0, 10.0])
                    .min_col_width((ui.available_width() - 24.0) / 3.0)
                    .show(ui, |ui| {
                        for (idx, s) in skills.iter().enumerate() {
                            Frame::none()
                                .fill(Color32::from_rgb(19, 23, 34))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(34, 41, 58)))
                                .rounding(Rounding::same(6.0))
                                .inner_margin(Margin::same(9.0))
                                .show(ui, |ui| {
                                    ui.set_min_width((ui.available_width() - 10.0).max(180.0));
                                    ui.horizontal(|ui| {
                                        if ui
                                            .link(
                                                RichText::new(&s.id)
                                                    .strong()
                                                    .size(12.5)
                                                    .color(Color32::from_rgb(165, 180, 252)),
                                            )
                                            .clicked()
                                        {
                                            clicked_skill = Some(s.id.clone());
                                        }
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if s.has_scripts {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "{} 脚本",
                                                            s.script_count
                                                        ))
                                                        .size(10.5)
                                                        .color(Color32::from_rgb(147, 197, 253)),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                    let short_desc: String =
                                        s.description.chars().take(52).collect();
                                    ui.label(
                                        RichText::new(short_desc)
                                            .size(11.0)
                                            .color(Color32::from_rgb(145, 155, 175)),
                                    );
                                });
                            if (idx + 1) % 3 == 0 {
                                ui.end_row();
                            }
                        }
                    });

                if let Some(sid) = clicked_skill {
                    self.load_skill_detail(sid);
                }
            }
        });
    }

    fn render_tab_console(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            card_frame().show(&mut cols[0], |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("执行 Lua").strong().color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("执行").color(Color32::WHITE))
                                    .fill(Color32::from_rgb(79, 70, 229)),
                            )
                            .clicked()
                        {
                            self.call_tool_async(
                                "execute_lua",
                                json!({ "code": self.lua_code }),
                                "执行 Lua",
                            );
                        }
                        if ui.small_button("GC 内存").clicked() {
                            self.lua_code =
                                "print('Lua KB:', collectgarbage('count'))".to_string();
                        }
                    });
                });
                ui.add_space(6.0);
                ui.add(
                    TextEdit::multiline(&mut self.lua_code)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(7)
                        .desired_width(f32::INFINITY),
                );

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("UI 点击模拟").size(12.0));
                    ui.add(
                        TextEdit::singleline(&mut self.ui_touch_path)
                            .hint_text("panel_1.button_1")
                            .desired_width(180.0),
                    );
                    if ui.button("左键").clicked() {
                        self.call_tool_async(
                            "trigger_ui_touch_event_by_path",
                            json!({ "ui_path": self.ui_touch_path, "touch": "左键" }),
                            "UI 左键",
                        );
                    }
                    if ui.button("右键").clicked() {
                        self.call_tool_async(
                            "trigger_ui_touch_event_by_path",
                            json!({ "ui_path": self.ui_touch_path, "touch": "右键" }),
                            "UI 右键",
                        );
                    }
                });
            });

            card_frame().show(&mut cols[1], |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("运行日志").strong().color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("刷新日志").clicked() {
                            self.load_logs();
                        }
                    });
                });
                ui.add_space(6.0);
                ui.add(
                    TextEdit::multiline(&mut self.game_logs)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(12)
                        .desired_width(f32::INFINITY),
                );
            });
        });
    }

    fn render_tab_table(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            card_frame().show(&mut cols[0], |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("table_type_combo")
                        .selected_text(&self.table_type)
                        .show_ui(ui, |ui| {
                            for (val, label) in [
                                ("unit", "unit (单位)"),
                                ("ability", "ability (技能)"),
                                ("modifier", "modifier (魔法效果)"),
                                ("item", "item (物品)"),
                                ("projectile", "projectile (投射物)"),
                                ("destructible", "destructible (可破坏物)"),
                                ("decoration", "decoration (装饰物)"),
                            ] {
                                if ui
                                    .selectable_value(&mut self.table_type, val.to_string(), label)
                                    .clicked()
                                {
                                    self.load_table_list();
                                }
                            }
                        });
                    if ui.button("读取列表").clicked() {
                        self.load_table_list();
                    }
                });
                ui.add_space(6.0);
                ui.add(
                    TextEdit::singleline(&mut self.table_search)
                        .hint_text("过滤名称或 Key...")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(6.0);

                let mut load_key = None;
                let kw = self.table_search.trim().to_lowercase();
                ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    for (k, n) in &self.table_items {
                        if !kw.is_empty()
                            && !k.to_string().contains(&kw)
                            && !n.to_lowercase().contains(&kw)
                        {
                            continue;
                        }
                        if ui
                            .selectable_label(
                                self.edit_obj_key == k.to_string(),
                                format!("[{}] {}", k, n),
                            )
                            .clicked()
                        {
                            load_key = Some(*k);
                        }
                    }
                });
                if let Some(k) = load_key {
                    self.load_table_item(k);
                }
            });

            card_frame().show(&mut cols[1], |ui| {
                ui.horizontal(|ui| {
                    ui.label("Key:");
                    ui.add(TextEdit::singleline(&mut self.edit_obj_key).desired_width(90.0));
                    ui.label("名称:");
                    ui.add(TextEdit::singleline(&mut self.edit_obj_name).desired_width(140.0));
                    if ui
                        .add(
                            egui::Button::new(RichText::new("保存").color(Color32::WHITE))
                                .fill(Color32::from_rgb(16, 185, 129)),
                        )
                        .clicked()
                    {
                        if let Ok(fields) = serde_json::from_str::<Value>(&self.edit_obj_json) {
                            let key_num = self.edit_obj_key.trim().parse::<u64>().ok();
                            self.call_tool_async(
                                "editor_table_upsert",
                                json!({
                                    "tableType": self.table_type,
                                    "key": key_num,
                                    "name": self.edit_obj_name,
                                    "fields": fields
                                }),
                                "保存物编",
                            );
                        } else {
                            self.set_toast("JSON 格式错误", true);
                        }
                    }
                });
                ui.add_space(6.0);
                ui.add(
                    TextEdit::multiline(&mut self.edit_obj_json)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(18)
                        .desired_width(f32::INFINITY),
                );
            });
        });
    }

    fn render_tab_ui_meta(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            card_frame().show(&mut cols[0], |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("UI 画布树").strong().color(Color32::WHITE));
                    ui.add(
                        TextEdit::singleline(&mut self.ui_node_path)
                            .hint_text("节点路径 (可空)")
                            .desired_width(130.0),
                    );
                    if ui.button("读取").clicked() {
                        let Some(pid) = self.active_project_id.clone() else {
                            return;
                        };
                        let map = self.active_map_name.clone();
                        let np = if self.ui_node_path.trim().is_empty() {
                            None
                        } else {
                            Some(self.ui_node_path.trim().to_string())
                        };
                        let depth = self.ui_depth;
                        let disp = self.dispatcher.clone();
                        let tx = self.tx.clone();
                        self.rt.spawn(async move {
                            if let Ok(res) = disp
                                .call_tool(
                                    "get_ui_canvas",
                                    &json!({
                                        "projectPath": pid,
                                        "mapName": map,
                                        "nodePath": np,
                                        "depth": depth
                                    }),
                                )
                                .await
                            {
                                let text = res
                                    .get("canvas")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| {
                                        serde_json::to_string_pretty(&res).unwrap_or_default()
                                    });
                                let _ = tx.send(AsyncMsg::UiTreeLoaded(text));
                            }
                        });
                    }
                });
                ui.add_space(6.0);
                ui.add(
                    TextEdit::multiline(&mut self.ui_tree_output)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(16)
                        .desired_width(f32::INFINITY),
                );
            });

            card_frame().show(&mut cols[1], |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("自定义数据与 Meta").strong().color(Color32::WHITE));
                    if ui.button("读取概览").clicked() {
                        let Some(pid) = self.active_project_id.clone() else {
                            return;
                        };
                        let map = self.active_map_name.clone();
                        let disp = self.dispatcher.clone();
                        let tx = self.tx.clone();
                        self.rt.spawn(async move {
                            if let Ok(res) = disp
                                .call_tool(
                                    "custom_define_overview",
                                    &json!({ "projectPath": pid, "mapName": map }),
                                )
                                .await
                            {
                                let _ = tx.send(AsyncMsg::MetaOutputLoaded(
                                    serde_json::to_string_pretty(&res).unwrap_or_default(),
                                ));
                            }
                        });
                    }
                    if ui.button("生成 Meta").clicked() {
                        self.call_tool_async("generate_meta", json!({}), "生成 Meta");
                    }
                });
                ui.add_space(6.0);
                ui.add(
                    TextEdit::multiline(&mut self.meta_output)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(16)
                        .desired_width(f32::INFINITY),
                );
            });
        });
    }

    fn render_tab_build(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            card_frame().show(&mut cols[0], |ui| {
                ui.label(RichText::new("ECA 编译 & GMP 打包").strong().color(Color32::WHITE));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.gmp_prefabs, "物编");
                    ui.checkbox(&mut self.gmp_ui, "UI");
                    ui.checkbox(&mut self.gmp_zh, "中文");
                    ui.checkbox(&mut self.gmp_us, "英文");
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("编译 ECA").color(Color32::WHITE))
                                .fill(Color32::from_rgb(79, 70, 229)),
                        )
                        .clicked()
                    {
                        self.call_tool_async("compile_eca", json!({}), "编译 ECA");
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("打包 .gmp").color(Color32::WHITE))
                                .fill(Color32::from_rgb(16, 185, 129)),
                        )
                        .clicked()
                    {
                        self.call_tool_async(
                            "save_gmp",
                            json!({
                                "updatePrefabs": self.gmp_prefabs,
                                "updateUI": self.gmp_ui,
                                "updateZHLanguage": self.gmp_zh,
                                "updateUSLanguage": self.gmp_us
                            }),
                            "打包 .gmp",
                        );
                    }
                    if ui.button("生成 Meta").clicked() {
                        self.call_tool_async("generate_meta", json!({}), "生成 Meta");
                    }
                    if ui.button("生成 LuaDoc").clicked() {
                        self.call_tool_async("generate_lua_doc", json!({}), "生成 LuaDoc");
                    }
                    if ui.button("启用全局脚本").clicked() {
                        self.call_tool_async("enable_global_script", json!({}), "启用全局脚本");
                    }
                    if ui
                        .button("初始化项目 (y3-lualib)")
                        .on_hover_text("克隆 y3-lualib 到 script/y3、复制项目配置并初始化 .y3maker")
                        .clicked()
                    {
                        self.init_current_project();
                    }
                });
            });

            card_frame().show(&mut cols[1], |ui| {
                ui.label(RichText::new("输出").strong().color(Color32::WHITE));
                ui.add_space(6.0);
                ui.add(
                    TextEdit::multiline(&mut self.build_output)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(10)
                        .desired_width(f32::INFINITY),
                );
            });
        });
    }

    fn render_tab_multi_mcp(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.label(RichText::new("三合一常驻 MCP 网关").strong().color(Color32::WHITE));
            ui.add_space(8.0);
            let p = self.mcp_port;
            ui.label(
                RichText::new(format!(
                    "• y3-helper / helper:  http://127.0.0.1:{}/mcp\n• y3editor (多编辑器路由):  http://127.0.0.1:{}/y3editor/mcp\n• y3runtime (常驻免刷新):  http://127.0.0.1:{}/y3runtime/mcp",
                    p, p, p
                ))
                .monospace()
                .size(12.0)
                .color(Color32::from_rgb(165, 180, 252)),
            );
        });

        ui.add_space(10.0);
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("在线编辑器实例 ({})", self.running_editors.len()))
                        .strong()
                        .color(Color32::WHITE),
                );
                if ui.small_button("扫描").clicked() {
                    let disp = self.dispatcher.clone();
                    let tx = self.tx.clone();
                    self.rt.spawn(async move {
                        if let Ok(res) = disp.call_tool("discover_editor_mcps", &json!({})).await {
                            let list = res
                                .get("editors")
                                .and_then(|v| v.as_array())
                                .cloned()
                                .unwrap_or_default();
                            let _ = tx.send(AsyncMsg::EditorMcpsLoaded(list));
                        }
                    });
                }
            });
            ui.add_space(6.0);
            if self.running_editors.is_empty() {
                ui.label(
                    RichText::new("暂无运行中的编辑器")
                        .size(12.0)
                        .color(Color32::from_rgb(130, 140, 160)),
                );
            } else {
                for ed in &self.running_editors {
                    let port = ed.get("port").and_then(|v| v.as_u64()).unwrap_or(8765);
                    let path = ed
                        .get("project_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let map = ed.get("map_name").and_then(|v| v.as_str()).unwrap_or("");
                    ui.label(
                        RichText::new(format!(":{}/mcp  —  {} ({})", port, path, map))
                            .monospace()
                            .color(Color32::from_rgb(110, 231, 183)),
                    );
                }
            }
        });
    }
}

fn card_frame() -> Frame {
    Frame::none()
        .fill(Color32::from_rgb(18, 22, 32))
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(32, 38, 54)))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(12.0))
}

fn status_pill(ui: &mut egui::Ui, text: &str, ok: bool) {
    let (fg, bg) = if ok {
        (
            Color32::from_rgb(110, 231, 183),
            Color32::from_rgb(14, 52, 40),
        )
    } else {
        (
            Color32::from_rgb(251, 191, 36),
            Color32::from_rgb(62, 42, 14),
        )
    };
    Frame::none()
        .fill(bg)
        .rounding(Rounding::same(4.0))
        .inner_margin(Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(fg));
        });
}

fn metric_chip(ui: &mut egui::Ui, k: &str, v: &str) {
    Frame::none()
        .fill(Color32::from_rgb(13, 16, 24))
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(30, 36, 50)))
        .rounding(Rounding::same(4.0))
        .inner_margin(Margin::symmetric(7.0, 3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}:", k))
                        .size(11.0)
                        .color(Color32::from_rgb(140, 150, 170)),
                );
                ui.label(RichText::new(v).strong().size(11.0).color(Color32::WHITE));
            });
        });
}

fn setup_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let font_candidates = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\simhei.ttf",
    ];
    for path in font_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("chinese".to_owned(), FontData::from_owned(bytes));
            if let Some(vec) = fonts.families.get_mut(&FontFamily::Proportional) {
                vec.insert(0, "chinese".to_owned());
            }
            if let Some(vec) = fonts.families.get_mut(&FontFamily::Monospace) {
                vec.push("chinese".to_owned());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn setup_dark_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(11, 13, 19);
    visuals.window_fill = Color32::from_rgb(16, 20, 29);
    visuals.extreme_bg_color = Color32::from_rgb(9, 11, 16);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(18, 22, 32);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(26, 32, 46);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(38, 46, 66);
    visuals.widgets.active.bg_fill = Color32::from_rgb(79, 70, 229);
    visuals.selection.bg_fill = Color32::from_rgb(67, 56, 202);
    ctx.set_visuals(visuals);
}

pub fn create_studio_icon_rgba(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let idx = (y * size + x) * 4;
            let nx = (x * 16) / size;
            let ny = (y * 16) / size;
            let in_bolt = (ny >= 2 && ny <= 8 && nx >= (9usize.saturating_sub(ny)) && nx <= 9)
                || (ny >= 7 && ny <= 13 && nx >= 6 && nx <= (16usize.saturating_sub(ny)));
            if in_bolt {
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            } else {
                rgba[idx] = 82;
                rgba[idx + 1] = 95;
                rgba[idx + 2] = 235;
                rgba[idx + 3] = 255;
            }
        }
    }
    rgba
}

/// 启动纯 Rust 原生 GPU/OpenGL 桌面客户端（0 WebView2 进程，内存占用 ~25MB）
pub fn run_native_desktop_gui(
    dispatcher: Arc<ToolDispatcher>,
    mcp_port: u16,
) -> anyhow::Result<()> {
    let icon = egui::IconData {
        rgba: create_studio_icon_rgba(32),
        width: 32,
        height: 32,
    };
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Y3 Studio")
            .with_icon(Arc::new(icon))
            .with_inner_size(Vec2::new(1280.0, 800.0))
            .with_min_inner_size(Vec2::new(980.0, 620.0)),
        ..Default::default()
    };

    eframe::run_native(
        "Y3 Studio",
        native_options,
        Box::new(move |cc| Ok(Box::new(Y3StudioApp::new(cc, dispatcher, mcp_port)))),
    )
    .map_err(|e| anyhow::anyhow!("启动原生 GUI 失败: {}", e))
}
