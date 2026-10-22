#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, Frame, Margin, RichText, Rounding,
    Stroke, Vec2,
};
use std::fs;
use std::io::{Cursor, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc};

const CREATE_NO_WINDOW: u32 = 0x08000000;
const APP_NAME: &str = "Y3 Studio";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const UNINSTALL_REG_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Y3Studio";

static PAYLOAD_ZIP: &[u8] = include_bytes!("../../target/installer-payload.zip");

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default();

    let is_uninstall = exe_name.contains("uninstall")
        || args.iter().any(|a| {
            let l = a.to_lowercase();
            l == "--uninstall" || l == "/u" || l == "/uninstall"
        });

    let is_silent = args.iter().any(|a| {
        let l = a.to_lowercase();
        l == "--silent" || l == "/s" || l == "/quiet" || l == "-s"
    });

    let mut custom_dir: Option<PathBuf> = None;
    for i in 0..args.len() {
        if (args[i] == "--dir" || args[i] == "/D") && i + 1 < args.len() {
            custom_dir = Some(PathBuf::from(&args[i + 1]));
        }
    }

    if is_silent {
        if is_uninstall {
            let dir = custom_dir.unwrap_or_else(detect_uninstall_dir);
            perform_uninstall(&dir)?;
        } else {
            let dir = custom_dir.unwrap_or_else(default_install_dir);
            let opts = InstallOptions {
                install_dir: dir,
                desktop_shortcut: true,
                start_menu_shortcut: true,
                add_to_path: true,
            };
            perform_install(&opts)?;
        }
        return Ok(());
    }

    let title = if is_uninstall {
        format!("{} v{} 卸载向导", APP_NAME, APP_VERSION)
    } else {
        format!("{} v{} 安装向导", APP_NAME, APP_VERSION)
    };

    let icon = egui::IconData {
        rgba: create_studio_icon_rgba(32),
        width: 32,
        height: 32,
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_icon(Arc::new(icon))
            .with_inner_size(Vec2::new(560.0, 420.0))
            .with_min_inner_size(Vec2::new(560.0, 420.0))
            .with_max_inner_size(Vec2::new(560.0, 420.0))
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };

    eframe::run_native(
        &title,
        native_options,
        Box::new(move |cc| {
            setup_fonts_and_theme(&cc.egui_ctx);
            Ok(Box::new(InstallerApp::new(is_uninstall, custom_dir)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("启动安装向导 GUI 失败: {}", e))
}

fn default_install_dir() -> PathBuf {
    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(local_appdata).join("Programs").join("Y3 Studio")
    } else {
        PathBuf::from(r"C:\Y3 Studio")
    }
}

fn detect_uninstall_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if parent.join("y3-helper.exe").exists() {
                return parent.to_path_buf();
            }
        }
    }
    default_install_dir()
}

#[derive(Clone)]
struct InstallOptions {
    install_dir: PathBuf,
    desktop_shortcut: bool,
    start_menu_shortcut: bool,
    add_to_path: bool,
}

#[derive(PartialEq, Clone)]
enum WizardStep {
    Configure,
    Working,
    Finished(Result<String, String>),
}

enum WorkerMsg {
    FolderPicked(String),
    Done(Result<String, String>),
}

struct InstallerApp {
    is_uninstall: bool,
    install_dir_str: String,
    desktop_shortcut: bool,
    start_menu_shortcut: bool,
    add_to_path: bool,
    launch_after_install: bool,
    step: WizardStep,
    tx: mpsc::Sender<WorkerMsg>,
    rx: mpsc::Receiver<WorkerMsg>,
}

impl InstallerApp {
    fn new(is_uninstall: bool, custom_dir: Option<PathBuf>) -> Self {
        let dir = custom_dir.unwrap_or_else(|| {
            if is_uninstall {
                detect_uninstall_dir()
            } else {
                default_install_dir()
            }
        });
        let (tx, rx) = mpsc::channel();
        Self {
            is_uninstall,
            install_dir_str: dir.to_string_lossy().to_string(),
            desktop_shortcut: true,
            start_menu_shortcut: true,
            add_to_path: true,
            launch_after_install: true,
            step: WizardStep::Configure,
            tx,
            rx,
        }
    }
}

impl eframe::App for InstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                WorkerMsg::FolderPicked(p) => {
                    let path = PathBuf::from(&p);
                    let final_path = if path
                        .file_name()
                        .map(|n| n.to_string_lossy().eq_ignore_ascii_case("Y3 Studio"))
                        .unwrap_or(false)
                    {
                        path
                    } else {
                        path.join("Y3 Studio")
                    };
                    self.install_dir_str = final_path.to_string_lossy().to_string();
                }
                WorkerMsg::Done(res) => {
                    self.step = WizardStep::Finished(res);
                }
            }
        }

        // 顶部品牌横幅
        egui::TopBottomPanel::top("installer_header")
            .exact_height(84.0)
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(16, 20, 32))
                    .inner_margin(Margin::symmetric(24.0, 18.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(32, 39, 58))),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // 闪电徽标卡片
                    Frame::none()
                        .fill(Color32::from_rgb(79, 70, 229))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(12.0, 8.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("⚡")
                                    .size(24.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                        });
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Y3 Studio")
                                    .size(20.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            Frame::none()
                                .fill(Color32::from_rgb(30, 41, 75))
                                .rounding(Rounding::same(4.0))
                                .inner_margin(Margin::symmetric(6.0, 2.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("v{} 正式版", APP_VERSION))
                                            .size(11.5)
                                            .color(Color32::from_rgb(165, 180, 252)),
                                    );
                                });
                        });
                        ui.add_space(3.0);
                        let subtitle = if self.is_uninstall {
                            "从您的计算机中移除 Y3 Studio 客户端与相关快捷方式"
                        } else {
                            "纯 Rust 原生 Y3 编辑器开发助手 & 三合一常驻 AI MCP 网关"
                        };
                        ui.label(
                            RichText::new(subtitle)
                                .size(12.0)
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                    });
                });
            });

        // 底部操作按钮栏
        egui::TopBottomPanel::bottom("installer_footer")
            .exact_height(60.0)
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(13, 16, 24))
                    .inner_margin(Margin::symmetric(24.0, 12.0))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(28, 34, 48))),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Y3 Studio Installer • 作者：不明不惑")
                            .size(11.5)
                            .color(Color32::from_rgb(100, 116, 139)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match &self.step {
                            WizardStep::Configure => {
                                let btn_text = if self.is_uninstall {
                                    "立即卸载"
                                } else {
                                    "立即安装"
                                };
                                let btn_color = if self.is_uninstall {
                                    Color32::from_rgb(220, 38, 38)
                                } else {
                                    Color32::from_rgb(79, 70, 229)
                                };

                                if ui
                                    .add_sized(
                                        [116.0, 34.0],
                                        egui::Button::new(
                                            RichText::new(btn_text)
                                                .size(13.5)
                                                .strong()
                                                .color(Color32::WHITE),
                                        )
                                        .fill(btn_color),
                                    )
                                    .clicked()
                                {
                                    self.step = WizardStep::Working;
                                    let tx = self.tx.clone();
                                    let egui_ctx = ctx.clone();
                                    let is_uninst = self.is_uninstall;
                                    let opts = InstallOptions {
                                        install_dir: PathBuf::from(self.install_dir_str.trim()),
                                        desktop_shortcut: self.desktop_shortcut,
                                        start_menu_shortcut: self.start_menu_shortcut,
                                        add_to_path: self.add_to_path,
                                    };
                                    std::thread::spawn(move || {
                                        let res = if is_uninst {
                                            perform_uninstall(&opts.install_dir)
                                                .map_err(|e| e.to_string())
                                        } else {
                                            perform_install(&opts).map_err(|e| e.to_string())
                                        };
                                        let _ = tx.send(WorkerMsg::Done(res));
                                        egui_ctx.request_repaint();
                                    });
                                }

                                ui.add_space(8.0);
                                if ui
                                    .add_sized(
                                        [84.0, 34.0],
                                        egui::Button::new(
                                            RichText::new("取消")
                                                .size(13.0)
                                                .color(Color32::from_rgb(203, 213, 225)),
                                        ),
                                    )
                                    .clicked()
                                {
                                    std::process::exit(0);
                                }
                            }
                            WizardStep::Working => {
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(RichText::new("正在处理...").size(13.0))
                                        .min_size(Vec2::new(116.0, 34.0)),
                                );
                            }
                            WizardStep::Finished(res) => {
                                if ui
                                    .add_sized(
                                        [116.0, 34.0],
                                        egui::Button::new(
                                            RichText::new("完成")
                                                .size(13.5)
                                                .strong()
                                                .color(Color32::WHITE),
                                        )
                                        .fill(Color32::from_rgb(16, 185, 129)),
                                    )
                                    .clicked()
                                {
                                    if self.is_uninstall {
                                        schedule_cleanup_self(Path::new(self.install_dir_str.trim()));
                                    } else if res.is_ok() && self.launch_after_install {
                                        let exe = PathBuf::from(self.install_dir_str.trim())
                                            .join("y3-helper.exe");
                                        if exe.exists() {
                                            let _ = Command::new(&exe)
                                                .current_dir(
                                                    exe.parent().unwrap_or_else(|| Path::new(".")),
                                                )
                                                .spawn();
                                        }
                                    }
                                    std::process::exit(0);
                                }
                            }
                        }
                    });
                });
            });

        // 中央主内容区
        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(11, 13, 19))
                    .inner_margin(Margin::symmetric(24.0, 18.0)),
            )
            .show(ctx, |ui| match &self.step {
                WizardStep::Configure => {
                    if self.is_uninstall {
                        Frame::none()
                            .fill(Color32::from_rgb(18, 22, 32))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 46, 66)))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(Margin::same(16.0))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("确认卸载 Y3 Studio")
                                        .size(14.5)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.add_space(10.0);
                                ui.label(
                                    RichText::new(format!(
                                        "即将从以下目录卸载 Y3 Studio v{}：\n{}",
                                        APP_VERSION, self.install_dir_str
                                    ))
                                    .size(12.5)
                                    .color(Color32::from_rgb(190, 200, 218)),
                                );
                                ui.add_space(10.0);
                                ui.label(
                                    RichText::new(
                                        "• 自动停止正在运行的 Y3 Studio 后台进程\n• 清理桌面与「开始」菜单快捷方式\n• 移除系统卸载注册表项与 PATH 环境变量配置",
                                    )
                                    .size(12.0)
                                    .color(Color32::from_rgb(148, 163, 184)),
                                );
                            });
                    } else {
                        ui.label(
                            RichText::new("安装位置")
                                .size(13.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [ui.available_width() - 82.0, 28.0],
                                egui::TextEdit::singleline(&mut self.install_dir_str),
                            );
                            if ui
                                .add_sized([74.0, 28.0], egui::Button::new("浏览..."))
                                .clicked()
                            {
                                let tx = self.tx.clone();
                                let egui_ctx = ctx.clone();
                                std::thread::spawn(move || {
                                    if let Some(p) = rfd::FileDialog::new()
                                        .set_title("选择 Y3 Studio 安装目录")
                                        .pick_folder()
                                    {
                                        let _ = tx.send(WorkerMsg::FolderPicked(
                                            p.to_string_lossy().to_string(),
                                        ));
                                        egui_ctx.request_repaint();
                                    }
                                });
                            }
                        });

                        ui.add_space(16.0);
                        ui.label(
                            RichText::new("安装选项")
                                .size(13.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.add_space(6.0);

                        Frame::none()
                            .fill(Color32::from_rgb(18, 22, 32))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(32, 38, 54)))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(Margin::same(14.0))
                            .show(ui, |ui| {
                                ui.checkbox(
                                    &mut self.desktop_shortcut,
                                    RichText::new("创建桌面快捷方式 (Y3 Studio)")
                                        .size(12.5)
                                        .color(Color32::from_rgb(226, 232, 240)),
                                );
                                ui.add_space(6.0);
                                ui.checkbox(
                                    &mut self.start_menu_shortcut,
                                    RichText::new("创建「开始」菜单程序快捷方式")
                                        .size(12.5)
                                        .color(Color32::from_rgb(226, 232, 240)),
                                );
                                ui.add_space(6.0);
                                ui.checkbox(
                                    &mut self.add_to_path,
                                    RichText::new(
                                        "将安装目录添加到用户 PATH 环境变量 (支持命令行调用 y3-helper)",
                                    )
                                    .size(12.5)
                                    .color(Color32::from_rgb(226, 232, 240)),
                                );
                            });
                    }
                }
                WizardStep::Working => {
                    ui.vertical_centered(|ui| {
                        ui.add_space(65.0);
                        ui.spinner();
                        ui.add_space(14.0);
                        let msg = if self.is_uninstall {
                            "正在卸载 Y3 Studio 并清理系统配置，请稍候..."
                        } else {
                            "正在释放程序文件并配置系统快捷方式，请稍候..."
                        };
                        ui.label(RichText::new(msg).size(14.0).color(Color32::WHITE));
                    });
                }
                WizardStep::Finished(res) => match res {
                    Ok(summary) => {
                        Frame::none()
                            .fill(Color32::from_rgb(14, 38, 32))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(16, 185, 129)))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(Margin::same(16.0))
                            .show(ui, |ui| {
                                let title = if self.is_uninstall {
                                    format!("✓ {} 已成功卸载", APP_NAME)
                                } else {
                                    format!("✓ {} v{} 安装完成！", APP_NAME, APP_VERSION)
                                };
                                ui.label(
                                    RichText::new(title)
                                        .size(15.0)
                                        .strong()
                                        .color(Color32::from_rgb(110, 231, 183)),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(summary)
                                        .size(12.0)
                                        .color(Color32::from_rgb(209, 250, 229)),
                                );
                            });

                        if !self.is_uninstall {
                            ui.add_space(16.0);
                            ui.checkbox(
                                &mut self.launch_after_install,
                                RichText::new("点击「完成」后立即启动 Y3 Studio")
                                    .size(13.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                        }
                    }
                    Err(err) => {
                        Frame::none()
                            .fill(Color32::from_rgb(45, 18, 24))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(239, 68, 68)))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(Margin::same(16.0))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("操作遇到错误")
                                        .size(15.0)
                                        .strong()
                                        .color(Color32::from_rgb(252, 165, 165)),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(err)
                                        .size(12.0)
                                        .color(Color32::from_rgb(254, 202, 202)),
                                );
                            });
                    }
                },
            });
    }
}

fn perform_install(opts: &InstallOptions) -> anyhow::Result<String> {
    let dir = &opts.install_dir;
    fs::create_dir_all(dir)?;

    // 1. 如目标目录下的 y3-helper.exe 正在运行，先关闭它以允许覆盖
    stop_running_instance_in_dir(dir);

    // 2. 解压内嵌的 PAYLOAD_ZIP 到安装目录
    let mut extracted_count = 0usize;
    if PAYLOAD_ZIP.len() > 22 {
        let reader = Cursor::new(PAYLOAD_ZIP);
        let mut archive = zip::ZipArchive::new(reader)?;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let Some(rel_path) = file.enclosed_name().map(|p| p.to_path_buf()) else {
                continue;
            };
            let out_path = dir.join(&rel_path);
            if file.is_dir() {
                fs::create_dir_all(&out_path)?;
            } else {
                if let Some(p) = out_path.parent() {
                    fs::create_dir_all(p)?;
                }
                let mut buf = Vec::with_capacity(file.size() as usize);
                file.read_to_end(&mut buf)?;
                fs::write(&out_path, &buf)?;
                extracted_count += 1;
            }
        }
    }

    // 若调试构建时 PAYLOAD_ZIP 为空，回退从当前目录查找 y3-helper.exe
    let target_exe = dir.join("y3-helper.exe");
    if !target_exe.exists() {
        if let Ok(cur_exe) = std::env::current_exe() {
            if let Some(parent) = cur_exe.parent() {
                let sibling = parent.join("y3-helper.exe");
                if sibling.exists() {
                    fs::copy(&sibling, &target_exe)?;
                    extracted_count += 1;
                }
            }
        }
    }

    if !target_exe.exists() {
        anyhow::bail!("未能在安装包中找到主程序 y3-helper.exe 载荷");
    }

    // 3. 生成标准 Windows 图标 y3-studio.ico
    let ico_path = dir.join("y3-studio.ico");
    fs::write(&ico_path, build_studio_ico_bytes())?;

    // 4. 将当前安装程序自身复制为 uninstall.exe
    let uninstaller_path = dir.join("uninstall.exe");
    if let Ok(self_exe) = std::env::current_exe() {
        if self_exe != uninstaller_path {
            let _ = fs::copy(&self_exe, &uninstaller_path);
        }
    }

    // 5. 创建桌面与开始菜单快捷方式
    if opts.desktop_shortcut {
        if let Some(desktop) = get_special_folder("Desktop") {
            let lnk = desktop.join("Y3 Studio.lnk");
            let _ = create_windows_shortcut(
                &lnk,
                &target_exe,
                "",
                dir,
                &ico_path,
                "Y3 Studio - Y3 编辑器开发助手与三合一常驻 MCP 网关",
            );
        }
    }

    if opts.start_menu_shortcut {
        if let Some(programs) = get_special_folder("Programs") {
            let folder = programs.join("Y3 Studio");
            let _ = fs::create_dir_all(&folder);
            let lnk = folder.join("Y3 Studio.lnk");
            let _ = create_windows_shortcut(
                &lnk,
                &target_exe,
                "",
                dir,
                &ico_path,
                "Y3 Studio - Y3 编辑器开发助手与三合一常驻 MCP 网关",
            );
            let uninst_lnk = folder.join("卸载 Y3 Studio.lnk");
            let _ = create_windows_shortcut(
                &uninst_lnk,
                &uninstaller_path,
                "--uninstall",
                dir,
                &ico_path,
                "卸载 Y3 Studio",
            );
        }
    }

    // 6. 注册 Windows 卸载注册表项与用户 PATH
    let exe_size_kb = (fs::metadata(&target_exe).map(|m| m.len()).unwrap_or(12_000_000) / 1024) as u32;
    register_uninstaller_in_registry(dir, &ico_path, &uninstaller_path, exe_size_kb);

    if opts.add_to_path {
        update_user_path(dir, true);
    }

    Ok(format!(
        "安装目录: {}\n已释放文件数: {} 个 (+ 卸载程序与快捷方式)\n您现在可以从桌面、「开始」菜单或命令行 (y3-helper) 启动程序。",
        dir.display(),
        extracted_count
    ))
}

fn perform_uninstall(dir: &Path) -> anyhow::Result<String> {
    stop_running_instance_in_dir(dir);

    // 1. 删除桌面快捷方式与开始菜单快捷方式
    if let Some(desktop) = get_special_folder("Desktop") {
        let _ = fs::remove_file(desktop.join("Y3 Studio.lnk"));
    }
    if let Some(programs) = get_special_folder("Programs") {
        let _ = fs::remove_dir_all(programs.join("Y3 Studio"));
    }

    // 2. 移除用户 PATH 与注册表卸载项
    update_user_path(dir, false);
    let _ = Command::new("reg")
        .args(["delete", UNINSTALL_REG_KEY, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    // 3. 删除安装目录中的文件
    for name in [
        "y3-helper.exe",
        "README.md",
        "使用说明.txt",
        "y3-studio.ico",
    ] {
        let p = dir.join(name);
        if p.exists() {
            let _ = fs::remove_file(p);
        }
    }

    // 4. 如果当前正在运行目录内的 uninstall.exe，启动后台延迟命令清理自身与空目录
    schedule_cleanup_self(dir);

    Ok(format!(
        "已从 {} 移除 Y3 Studio 程序文件、快捷方式及系统注册表项。",
        dir.display()
    ))
}

fn schedule_cleanup_self(dir: &Path) {
    let uninst_exe = dir.join("uninstall.exe");
    if uninst_exe.exists() && fs::remove_file(&uninst_exe).is_err() {
        let cmd_line = format!(
            "ping 127.0.0.1 -n 2 > nul & del /f /q \"{}\" & rmdir \"{}\"",
            uninst_exe.display(),
            dir.display()
        );
        let _ = Command::new("cmd")
            .current_dir(std::env::temp_dir())
            .args(["/C", &cmd_line])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    } else {
        let _ = fs::remove_dir(dir);
    }
}

fn stop_running_instance_in_dir(dir: &Path) {
    let target_exe = dir.join("y3-helper.exe");
    if !target_exe.exists() {
        return;
    }
    let escaped = target_exe.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "Get-Process y3-helper -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -eq '{}' }} | Stop-Process -Force",
        escaped
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    std::thread::sleep(std::time::Duration::from_millis(250));
}

fn get_special_folder(name: &str) -> Option<PathBuf> {
    let ps = format!("[Environment]::GetFolderPath('{}')", name);
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path_str.is_empty() {
        None
    } else {
        Some(PathBuf::from(path_str))
    }
}

fn create_windows_shortcut(
    lnk_path: &Path,
    target_path: &Path,
    arguments: &str,
    working_dir: &Path,
    icon_path: &Path,
    description: &str,
) -> anyhow::Result<()> {
    let esc = |s: &str| s.replace('\'', "''");
    let script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{}'); \
         $s.TargetPath = '{}'; \
         $s.Arguments = '{}'; \
         $s.WorkingDirectory = '{}'; \
         $s.IconLocation = '{},0'; \
         $s.Description = '{}'; \
         $s.Save()",
        esc(&lnk_path.to_string_lossy()),
        esc(&target_path.to_string_lossy()),
        esc(arguments),
        esc(&working_dir.to_string_lossy()),
        esc(&icon_path.to_string_lossy()),
        esc(description),
    );
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    Ok(())
}

fn register_uninstaller_in_registry(
    install_dir: &Path,
    icon_path: &Path,
    uninstaller_path: &Path,
    size_kb: u32,
) {
    let dir_str = install_dir.to_string_lossy().to_string();
    let icon_str = icon_path.to_string_lossy().to_string();
    let uninst_cmd = format!("\"{}\" --uninstall", uninstaller_path.display());
    let quiet_uninst_cmd = format!("\"{}\" --uninstall --silent", uninstaller_path.display());

    let str_entries = [
        ("DisplayName", APP_NAME),
        ("DisplayVersion", APP_VERSION),
        ("Publisher", "不明不惑"),
        ("InstallLocation", &dir_str),
        ("DisplayIcon", &icon_str),
        ("UninstallString", &uninst_cmd),
        ("QuietUninstallString", &quiet_uninst_cmd),
    ];

    for (k, v) in str_entries {
        let _ = Command::new("reg")
            .args(["add", UNINSTALL_REG_KEY, "/v", k, "/t", "REG_SZ", "/d", v, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }

    let dword_entries = [
        ("NoModify", "1"),
        ("NoRepair", "1"),
        ("EstimatedSize", &size_kb.to_string()),
    ];
    for (k, v) in dword_entries {
        let _ = Command::new("reg")
            .args([
                "add",
                UNINSTALL_REG_KEY,
                "/v",
                k,
                "/t",
                "REG_DWORD",
                "/d",
                v,
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
}

fn update_user_path(install_dir: &Path, add: bool) {
    let dir_str = install_dir.to_string_lossy().replace('\'', "''");
    let script = if add {
        format!(
            "$p = [Environment]::GetEnvironmentVariable('Path', 'User'); \
             if (-not $p) {{ $p = '' }}; \
             $items = $p -split ';' | Where-Object {{ $_ -ne '' }}; \
             if ($items -notcontains '{}') {{ \
                 $newPath = ($items + '{}') -join ';'; \
                 [Environment]::SetEnvironmentVariable('Path', $newPath, 'User'); \
             }}",
            dir_str, dir_str
        )
    } else {
        format!(
            "$p = [Environment]::GetEnvironmentVariable('Path', 'User'); \
             if ($p) {{ \
                 $items = $p -split ';' | Where-Object {{ $_ -ne '' -and $_ -ne '{}' }}; \
                 $newPath = $items -join ';'; \
                 [Environment]::SetEnvironmentVariable('Path', $newPath, 'User'); \
             }}",
            dir_str
        )
    };
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

fn create_studio_icon_rgba(size: usize) -> Vec<u8> {
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

/// 生成标准 32x32 32-bit BGRA Windows `.ico` 图标文件字节流
fn build_studio_ico_bytes() -> Vec<u8> {
    let w = 32usize;
    let h = 32usize;
    let rgba = create_studio_icon_rgba(w);
    let xor_size = w * h * 4;
    let and_size = (w * h) / 8;
    let dib_size = 40 + xor_size + and_size;

    let mut ico = Vec::with_capacity(22 + dib_size);
    // ICONDIR (6 bytes)
    ico.extend_from_slice(&0u16.to_le_bytes()); // Reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // Type = 1 (ICO)
    ico.extend_from_slice(&1u16.to_le_bytes()); // Count = 1

    // ICONDIRENTRY (16 bytes)
    ico.push(w as u8);
    ico.push(h as u8);
    ico.push(0); // ColorCount
    ico.push(0); // Reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // Planes = 1
    ico.extend_from_slice(&32u16.to_le_bytes()); // BitCount = 32
    ico.extend_from_slice(&(dib_size as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // Offset = 22

    // BITMAPINFOHEADER (40 bytes)
    ico.extend_from_slice(&40u32.to_le_bytes()); // biSize
    ico.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    ico.extend_from_slice(&((h * 2) as i32).to_le_bytes()); // biHeight (XOR + AND)
    ico.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    ico.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    ico.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    ico.extend_from_slice(&((xor_size + and_size) as u32).to_le_bytes()); // biSizeImage
    ico.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    ico.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    ico.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    ico.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    // XOR Mask (Bottom-Up BGRA)
    for y in (0..h).rev() {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            ico.push(rgba[idx + 2]); // B
            ico.push(rgba[idx + 1]); // G
            ico.push(rgba[idx]); // R
            ico.push(rgba[idx + 3]); // A
        }
    }

    // AND Mask (all 0 = opaque)
    ico.extend(std::iter::repeat(0u8).take(and_size));
    ico
}

fn setup_fonts_and_theme(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for path in [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\simhei.ttf",
    ] {
        if let Ok(bytes) = fs::read(path) {
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

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(11, 13, 19);
    visuals.window_fill = Color32::from_rgb(16, 20, 29);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(18, 22, 32);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(26, 32, 46);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(38, 46, 66);
    visuals.widgets.active.bg_fill = Color32::from_rgb(79, 70, 229);
    ctx.set_visuals(visuals);
}
