use serde_json::json;
use std::fs;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use y3_helper::{
    console::ConsoleProtocol,
    gmp::{GmpHeader, GmpParser},
    server::ToolDispatcher,
    workspace::MultiProjectManager,
};

fn create_mock_y3_project(
    base_dir: &std::path::Path,
    project_name: &str,
    entry_id: u64,
    extra_maps: &[(&str, u64)],
) -> std::path::PathBuf {
    let proj_dir = base_dir.join(project_name);
    let entry_dir = proj_dir.join("maps/EntryMap");
    fs::create_dir_all(entry_dir.join("script/y3-helper/plugin")).unwrap();
    fs::create_dir_all(entry_dir.join("global_trigger/trigger")).unwrap();
    fs::create_dir_all(entry_dir.join("global_trigger/function")).unwrap();
    fs::create_dir_all(entry_dir.join("ui/prefab")).unwrap();

    fs::write(
        proj_dir.join("header.project"),
        json!({ "entry_map": { "id": entry_id } }).to_string(),
    )
    .unwrap();
    fs::write(
        proj_dir.join("setting.json"),
        json!({
            "use_main_level_trigger_and_object": false,
            "use_main_level_ui": false
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        entry_dir.join("header.map"),
        json!({ "id": entry_id }).to_string(),
    )
    .unwrap();
    fs::write(
        entry_dir.join("desc.json"),
        json!({ "name": format!("({}_主地图)", project_name) }).to_string(),
    )
    .unwrap();

    // 写入初始空 .gmp 文件
    let gmp = GmpParser {
        header: GmpHeader {
            uuid: [0x11; 16],
            version: 5,
            pack_size: 2,
            create_stamp: "20261021".into(),
            editor_pack_stamp: "20261021".into(),
        },
        sections: Default::default(),
    };
    fs::write(entry_dir.join("EntryMap.gmp"), gmp.build_bytes()).unwrap();

    // 写入 UI 画布与 SceneUI
    fs::write(
        entry_dir.join("ui/MainHUD.json"),
        json!({
            "name": "MainHUD",
            "uid": "canvas_1",
            "type": 7,
            "childs": [
                {
                    "name": "StartBtn",
                    "uid": "btn_start_01",
                    "type": 1,
                    "childs": []
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        entry_dir.join("ui/SceneUI.json"),
        json!({
            "items": {
                "hp_bar": { "name": "头顶血条", "uid": "scene_hp_01" }
            }
        })
        .to_string(),
    )
    .unwrap();

    // 写入自定义单位属性与自定义事件
    fs::write(
        entry_dir.join("attr.json"),
        json!({
            "user_specified": [[{"key": "magic_power", "name": "法术强度"}]]
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        entry_dir.join("customevent.json"),
        json!({
            "conf": {
                "1001": {
                    "id": 1001,
                    "name": "波次开始",
                    "args": [[{"desc": "当前波数", "type": 100000}]]
                }
            },
            "group_info": [[2, 1001]]
        })
        .to_string(),
    )
    .unwrap();

    // 写入 ECA 触发器
    fs::write(
        entry_dir.join("global_trigger/trigger/InitTrigger.json"),
        json!({
            "enable": true,
            "event": [{"type": "EVENT_GAME_INIT", "args": []}],
            "root": {
                "actions": [
                    {
                        "type": "PRINT_MESSAGE_ACTION_TO_DIALOG",
                        "args": [
                            {"type": "100175", "value": "3"},
                            {"type": "STR", "value": format!("Welcome to {}", project_name)}
                        ]
                    }
                ]
            }
        })
        .to_string(),
    )
    .unwrap();

    for &(sub_name, sub_id) in extra_maps {
        let sub_dir = proj_dir.join("maps").join(sub_name);
        fs::create_dir_all(sub_dir.join("script")).unwrap();
        fs::write(
            sub_dir.join("header.map"),
            json!({ "id": sub_id }).to_string(),
        )
        .unwrap();
        fs::write(
            sub_dir.join("desc.json"),
            json!({ "name": format!("({})", sub_name) }).to_string(),
        )
        .unwrap();
        fs::write(sub_dir.join(format!("{}.gmp", sub_name)), gmp.build_bytes()).unwrap();
    }

    proj_dir
}

#[tokio::test]
async fn test_end_to_end_multi_project_and_multi_map_mcp_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    let proj_rpg = create_mock_y3_project(
        tmp.path(),
        "RPGProject",
        10001,
        &[("BossArena", 10002), ("TownMap", 10003)],
    );
    let proj_td = create_mock_y3_project(
        tmp.path(),
        "TowerDefenseProject",
        20001,
        &[("EndlessWaveMap", 20002)],
    );

    let mgr = MultiProjectManager::new(None, false);
    let dispatcher = ToolDispatcher::new(mgr.clone());

    // 1. 通过 MCP 工具同时注册两个项目
    let reg_rpg = dispatcher
        .call_tool(
            "set_project_path",
            &json!({ "projectPath": proj_rpg.to_string_lossy(), "setActive": true }),
        )
        .await
        .unwrap();
    let reg_td = dispatcher
        .call_tool(
            "set_project_path",
            &json!({ "projectPath": proj_td.to_string_lossy(), "setActive": false }),
        )
        .await
        .unwrap();

    let port_rpg = reg_rpg["console_port"].as_u64().unwrap() as u16;
    let port_td = reg_td["console_port"].as_u64().unwrap() as u16;
    assert_ne!(port_rpg, port_td);

    // 2. 同时向 RPGProject 的 BossArena 地图和 TowerDefenseProject 的 EndlessWaveMap 地图写入不同的物编数据
    let upsert_rpg = dispatcher
        .call_tool(
            "editor_table_upsert",
            &json!({
                "projectPath": proj_rpg.to_string_lossy(),
                "mapName": "BossArena",
                "tableType": "unit",
                "key": 134250001,
                "name": "深渊魔王",
                "fields": { "ori_bits": 99 },
                "kv": { "boss_tier": 5 }
            }),
        )
        .await
        .unwrap();
    assert_eq!(upsert_rpg["name"], "深渊魔王");

    let upsert_td = dispatcher
        .call_tool(
            "editor_table_upsert",
            &json!({
                "projectPath": proj_td.to_string_lossy(),
                "mapName": "EndlessWaveMap",
                "tableType": "ability",
                "key": 134260001,
                "name": "连锁闪电塔技能",
                "fields": { "ability_cost": 50 }
            }),
        )
        .await
        .unwrap();
    assert_eq!(upsert_td["name"], "连锁闪电塔技能");

    // 验证两个项目各子地图的物编互不污染
    let rpg_units = dispatcher
        .call_tool(
            "editor_table_list",
            &json!({
                "projectPath": proj_rpg.to_string_lossy(),
                "mapName": "BossArena",
                "tableType": "unit"
            }),
        )
        .await
        .unwrap();
    assert_eq!(rpg_units["count"], 1);
    assert_eq!(rpg_units["items"][0]["name"], "深渊魔王");

    let td_abilities = dispatcher
        .call_tool(
            "editor_table_list",
            &json!({
                "projectPath": proj_td.to_string_lossy(),
                "mapName": "EndlessWaveMap",
                "tableType": "ability"
            }),
        )
        .await
        .unwrap();
    assert_eq!(td_abilities["count"], 1);
    assert_eq!(td_abilities["items"][0]["name"], "连锁闪电塔技能");

    // 3. 并发编译两个项目的 ECA 触发器、生成 Meta 与打包 GMP
    let eca_rpg = dispatcher
        .call_tool(
            "compile_eca",
            &json!({ "projectPath": proj_rpg.to_string_lossy(), "mapName": "EntryMap" }),
        )
        .await
        .unwrap();
    assert_eq!(eca_rpg["trigger_count"], 1);

    let gmp_rpg = dispatcher
        .call_tool(
            "save_gmp",
            &json!({ "projectPath": proj_rpg.to_string_lossy(), "mapName": "BossArena" }),
        )
        .await
        .unwrap();
    assert_eq!(gmp_rpg["success"], true);

    // 4. 查询 UI 画布树
    let ui_res = dispatcher
        .call_tool(
            "get_ui_canvas",
            &json!({ "projectPath": proj_rpg.to_string_lossy(), "mapName": "EntryMap", "depth": 2 }),
        )
        .await
        .unwrap();
    assert_eq!(ui_res["success"], true);
    assert!(ui_res["canvas"].as_str().unwrap().contains("StartBtn [Button]"));

    // 5. 模拟两个项目的游戏客户端同时连接到各自的 ConsoleServer 端口，验证隔离通信
    let mut client_rpg = TcpStream::connect(("127.0.0.1", port_rpg)).await.unwrap();
    let mut client_td = TcpStream::connect(("127.0.0.1", port_td)).await.unwrap();

    client_rpg
        .write_all(&ConsoleProtocol::encode(&json!({
            "method": "updatePlayer",
            "params": { "name": "RPG窗口1", "multiMode": false }
        })))
        .await
        .unwrap();
    client_rpg
        .write_all(&ConsoleProtocol::encode(&json!({
            "method": "print",
            "params": { "message": "RPG_LOG_LINE_1" }
        })))
        .await
        .unwrap();

    client_td
        .write_all(&ConsoleProtocol::encode(&json!({
            "method": "updatePlayer",
            "params": { "name": "TD窗口1", "multiMode": false }
        })))
        .await
        .unwrap();
    client_td
        .write_all(&ConsoleProtocol::encode(&json!({
            "method": "print",
            "params": { "message": "TD_LOG_LINE_1" }
        })))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let status_rpg = dispatcher
        .call_tool(
            "get_game_status",
            &json!({ "projectPath": proj_rpg.to_string_lossy() }),
        )
        .await
        .unwrap();
    assert_eq!(status_rpg["running"], true);
    assert_eq!(status_rpg["clients"][0]["name"], "RPG窗口1");

    let logs_td = dispatcher
        .call_tool(
            "get_logs",
            &json!({ "projectPath": proj_td.to_string_lossy() }),
        )
        .await
        .unwrap();
    assert!(logs_td["logs"].as_str().unwrap().contains("TD_LOG_LINE_1"));
    assert!(!logs_td["logs"].as_str().unwrap().contains("RPG_LOG_LINE_1"));
}
