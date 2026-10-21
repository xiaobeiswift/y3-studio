#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use serde_json::{json, Map, Value};
use y3_helper::{
    cli::{Cli, Commands},
    custom_define::CustomDefine,
    eca_compiler::EcaCompiler,
    editor_table::{
        excel::{apply_excel_rule, ExcelImportRule},
        table::{EditorTable, TableType},
    },
    env::ProjectContext,
    features::{
        cloud_script::CloudScriptManager,
        global_script::GlobalScriptManager,
        lua_doc::LuaDocGenerator,
        network_server::NetworkTestServer,
        plugin::PluginManager,
        project_init::{ProjectInitializer, RepoSource},
    },
    gmp::{save_gmp, GmpSaveOptions},
    gui::{restore_saved_projects, run_native_desktop_gui},
    launcher::{GameLauncher, LaunchGameOptions},
    meta_builder::MetaBuilder,
    server::{run_stdio_mcp_server, start_http_server, ToolDispatcher},
    workspace::MultiProjectManager,
};

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Commands::Gui {
        port: 8766,
        projects: Vec::new(),
    });

    // 若不是 Stdio MCP 或 Agent 模式，初始化日志输出到 stderr
    if !matches!(command, Commands::McpStdio { .. } | Commands::Agent { .. }) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_writer(std::io::stderr)
            .try_init();
    }

    match command {
        Commands::Gui { port, projects } => {
            let mgr = MultiProjectManager::new(cli.editor_path.as_deref(), true);
            if projects.is_empty() {
                if let Ok(cwd) = std::env::current_dir() {
                    let _ = mgr.register_project(&cwd, true).await;
                }
            } else {
                for (idx, p) in projects.iter().enumerate() {
                    let _ = mgr.register_project(p, idx == 0).await;
                }
            }
            let dispatcher = ToolDispatcher::new(mgr);
            restore_saved_projects(&dispatcher).await;
            let mut bound_port = None;
            // 注意：8765 是 Y3 编辑器内置 y3editor MCP 默认端口，8767 是游戏运行模式 y3runtime MCP 默认端口
            for candidate in [port, 8768, 18766, 0] {
                if let Ok(actual_port) = start_http_server(dispatcher.clone(), candidate).await {
                    bound_port = Some(actual_port);
                    break;
                }
            }
            let actual_port = bound_port.ok_or_else(|| anyhow::anyhow!("无法绑定本地 HTTP 端口"))?;
            eprintln!(
                "[Y3-Helper] 原生桌面客户端已启动 (内置 MCP 网关: http://127.0.0.1:{}/mcp | /y3editor/mcp | /y3runtime/mcp)",
                actual_port
            );
            run_native_desktop_gui(dispatcher, actual_port)?;
        }

        Commands::Serve {
            port,
            projects,
            network_test_server,
        } => {
            let mgr = MultiProjectManager::new(cli.editor_path.as_deref(), true);
            for (idx, p) in projects.iter().enumerate() {
                match mgr.register_project(p, idx == 0).await {
                    Ok(ws) => {
                        eprintln!(
                            "[Y3-Helper] 已挂载项目 [{}] {} (Console 端口: {})",
                            ws.project_id,
                            ws.project_path.display(),
                            ws.console_server.port
                        );
                    }
                    Err(e) => {
                        eprintln!("[Y3-Helper] 挂载项目 {} 失败: {}", p.display(), e);
                    }
                }
            }
            if network_test_server {
                let (p1, p2) = NetworkTestServer::start_both().await?;
                eprintln!("[Y3-Helper] 网络测试服务器已启动: 127.0.0.1:{}, 127.0.0.1:{}", p1, p2);
            }
            let dispatcher = ToolDispatcher::new(mgr);
            let actual_port = start_http_server(dispatcher, port).await?;
            eprintln!(
                "[Y3-Helper] 多项目独立服务已启动: http://127.0.0.1:{}/mcp (健康检查: http://127.0.0.1:{}/health)",
                actual_port, actual_port
            );
            tokio::signal::ctrl_c().await?;
        }

        Commands::McpStdio { projects } => {
            let mgr = MultiProjectManager::new(cli.editor_path.as_deref(), true);
            for (idx, p) in projects.iter().enumerate() {
                let _ = mgr.register_project(p, idx == 0).await;
            }
            let dispatcher = ToolDispatcher::new(mgr);
            run_stdio_mcp_server(dispatcher).await?;
        }

        Commands::Status { projects } => {
            let mgr = MultiProjectManager::new(cli.editor_path.as_deref(), false);
            if projects.is_empty() {
                if let Ok(cwd) = std::env::current_dir() {
                    let _ = mgr.register_project(&cwd, true).await;
                }
            } else {
                for (idx, p) in projects.iter().enumerate() {
                    let _ = mgr.register_project(p, idx == 0).await;
                }
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&mgr.list_projects_json())?
            );
        }

        Commands::Init {
            project,
            map,
            source,
        } => {
            let mgr = MultiProjectManager::new(cli.editor_path.as_deref(), false);
            let ctx = ProjectContext::load_from_path(&project)?;
            let repo = RepoSource::parse(Some(&source));
            let ed = mgr.editor_env();
            let rep = ProjectInitializer::init_map(&ctx, map.as_deref(), repo, ed.as_ref(), true)?;
            println!("{}", serde_json::to_string_pretty(&rep)?);
        }

        Commands::LaunchGame {
            project,
            map,
            multi,
            players,
            tracy,
        } => {
            let mgr = MultiProjectManager::new(cli.editor_path.as_deref(), false);
            let ws = mgr.register_project(&project, true).await?;
            let ed = mgr.editor_env();
            let ctx = ws.context.read().clone();
            let opts = LaunchGameOptions {
                map_name: map,
                multi_mode: multi,
                multi_players: players,
                tracy,
                lua_args: Default::default(),
                save_gmp_before_launch: Some(true),
            };
            let res = ws.session_manager.launch_game(&ctx, ed.as_ref(), opts).await;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }

        Commands::LaunchEditor { project } => {
            let mgr = MultiProjectManager::new(cli.editor_path.as_deref(), false);
            let ctx = ProjectContext::load_from_path(&project)?;
            let ed = mgr
                .editor_env()
                .ok_or_else(|| anyhow::anyhow!("未检测到 Y3 编辑器路径"))?;
            let pid = GameLauncher::launch_editor(&ctx, &ed)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": true,
                    "project_id": ctx.project_id,
                    "editor_pid": pid
                }))?
            );
        }

        Commands::KillGame { project, all } => {
            let proj_path = if let Some(p) = project {
                Some(
                    ProjectContext::load_from_path(&p)
                        .map(|c| c.project_path)
                        .unwrap_or(p),
                )
            } else {
                None
            };
            let killed = GameLauncher::kill_game(proj_path.as_deref(), &[], all)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": true,
                    "killed_pids": killed
                }))?
            );
        }

        Commands::CompileEca {
            project,
            map,
            override_dir,
        } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            let rep =
                EcaCompiler::compile_map(&ctx, map.as_deref(), override_dir.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&rep)?);
        }

        Commands::GenerateMeta { project, map } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            if let Some(m) = map {
                let rep = MetaBuilder::build_for_map(&ctx, Some(&m))?;
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                let reps = MetaBuilder::build_all_maps(&ctx)?;
                println!("{}", serde_json::to_string_pretty(&reps)?);
            }
        }

        Commands::SaveGmp { project, map } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            let trig_map = ctx.trigger_map_for(map.as_deref())?;
            let res = save_gmp(&trig_map.map_path, GmpSaveOptions::default())?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }

        Commands::Table {
            project,
            map,
            table_type,
            action,
            key,
            name,
            fields_json,
            kv_json,
        } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            let tt = TableType::parse(&table_type)
                .ok_or_else(|| anyhow::anyhow!("未知物编表类型: {}", table_type))?;
            let table = EditorTable::open(&ctx, map.as_deref(), tt)?;

            match action.as_str() {
                "list" => {
                    let list = table.list_summaries()?;
                    println!("{}", serde_json::to_string_pretty(&list)?);
                }
                "get" => {
                    let k = key.ok_or_else(|| anyhow::anyhow!("get 操作需要 --key"))?;
                    let obj = table.get(k)?;
                    println!("{}", serde_json::to_string_pretty(&obj.json_data)?);
                }
                "upsert" => {
                    let fields: Option<Map<String, Value>> = match fields_json {
                        Some(s) => serde_json::from_str(&s)?,
                        None => None,
                    };
                    let kv: Option<Map<String, Value>> = match kv_json {
                        Some(s) => serde_json::from_str(&s)?,
                        None => None,
                    };
                    let obj = if let Some(k) = key {
                        if table.list_keys()?.contains(&k) {
                            let mut fm = fields.unwrap_or_default();
                            if let Some(n) = name {
                                fm.insert("name".to_string(), Value::String(n));
                            }
                            table.update(k, Some(&fm), kv.as_ref())?
                        } else {
                            table.create(
                                Some(k),
                                name.as_deref().unwrap_or("新建物编"),
                                None,
                                fields.as_ref(),
                                kv.as_ref(),
                            )?
                        }
                    } else {
                        table.create(
                            None,
                            name.as_deref().unwrap_or("新建物编"),
                            None,
                            fields.as_ref(),
                            kv.as_ref(),
                        )?
                    };
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": true,
                            "key": obj.key,
                            "name": obj.name,
                            "file_path": obj.file_path
                        }))?
                    );
                }
                "delete" => {
                    let k = key.ok_or_else(|| anyhow::anyhow!("delete 操作需要 --key"))?;
                    let ok = table.delete(k)?;
                    println!("{}", serde_json::to_string_pretty(&json!({ "deleted": ok, "key": k }))?);
                }
                other => anyhow::bail!("不支持的 Table action: {}", other),
            }
        }

        Commands::ExcelImport {
            project,
            map,
            table_type,
            excel,
            sheet,
        } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            let rule = ExcelImportRule {
                table_type,
                excel_path: excel,
                sheet_name: sheet,
                offset: None,
                skip_rows: 0,
                template_key: None,
                field_map: Default::default(),
                kv_columns: Default::default(),
            };
            let res = apply_excel_rule(&ctx, map.as_deref(), &rule)?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }

        Commands::GlobalScript { project } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            let files = GlobalScriptManager::enable(&ctx)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": true,
                    "updated_files": files
                }))?
            );
        }

        Commands::CloudScript {
            project,
            enable,
            wait_debugger,
        } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            let p = CloudScriptManager::sync_cloud_script(&ctx, enable, None, None, wait_debugger)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": true,
                    "cloud_script_path": p
                }))?
            );
        }

        Commands::Plugin {
            project,
            map,
            action,
            func,
        } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            match action.as_str() {
                "init" => {
                    let dir = PluginManager::init_plugins(&ctx, map.as_deref())?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({ "success": true, "plugin_dir": dir }))?
                    );
                }
                "list" => {
                    let list = PluginManager::list_plugins(&ctx, map.as_deref())?;
                    println!("{}", serde_json::to_string_pretty(&list)?);
                }
                "run" => {
                    let out = PluginManager::run_plugin_function(&ctx, map.as_deref(), None, &func)?;
                    println!("{}", serde_json::to_string_pretty(&json!({ "outputs": out }))?);
                }
                other => anyhow::bail!("不支持的 Plugin action: {}", other),
            }
        }

        Commands::UiCanvas {
            project,
            map,
            node_path,
            depth,
        } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            let pkg = CustomDefine::get_ui_package(&ctx, map.as_deref())?;
            let max_depth = if depth < 0 { None } else { Some(depth as usize) };
            let tree =
                CustomDefine::format_ui_canvas_tree(&pkg, node_path.as_deref(), max_depth)?;
            println!("{}", tree);
        }

        Commands::LuaDoc { project, map } => {
            let ctx = ProjectContext::load_from_path(&project)?;
            let files = LuaDocGenerator::generate_from_doc_json(&ctx, map.as_deref(), None)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "generated_files": files }))?
            );
        }

        Commands::NetworkServer => {
            let (p1, p2) = NetworkTestServer::start_both().await?;
            println!(
                "Y3 网络测试服务器已启动:\n- 回显服务: 127.0.0.1:{}\n- 定时推送服务: 127.0.0.1:{}",
                p1, p2
            );
            tokio::signal::ctrl_c().await?;
        }

        Commands::Agent { allowed_exe } => {
            GameLauncher::run_elevated_agent_stdio_loop(&allowed_exe)?;
        }
    }

    Ok(())
}
