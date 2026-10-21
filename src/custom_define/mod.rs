use crate::env::ProjectContext;
use crate::utils::y3json::convert_tuple_format;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitAttr {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerAttr {
    pub key: String,
    pub name: String,
    pub desc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEventArg {
    pub name: String,
    pub lua_type: String,
    pub desc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEventItem {
    pub id: i64,
    pub name: String,
    pub path: Vec<String>,
    pub args: Vec<CustomEventArg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UINode {
    pub name: String,
    pub uid: String,
    pub node_type: i64,
    pub childs: Vec<UINode>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UIPackage {
    pub canvases: Vec<UINode>,
    pub prefabs: Vec<UINode>,
    pub scene_uis: Vec<UINode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIAnimItem {
    pub uid: String,
    pub name: String,
    pub frame: i64,
    pub time: f64,
    pub play_mode: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpWordItem {
    pub uid: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontItem {
    pub uid: String,
    pub name: String,
}

/// 解析指定项目、指定子地图下的 7 类自定义枚举数据
pub struct CustomDefine;

impl CustomDefine {
    /// 1. 单位属性 (`attr.json`)
    pub fn get_unit_attrs(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Vec<UnitAttr>> {
        let trigger_map = project.trigger_map_for(map_name)?;
        let file_path = trigger_map.map_path.join("attr.json");
        if !file_path.is_file() {
            return Ok(Vec::new());
        }
        let raw: Value = serde_json::from_str(&fs::read_to_string(&file_path)?)?;
        let plain = convert_tuple_format(&raw);
        let mut attrs = Vec::new();

        if let Some(arr) = plain.get("user_specified").and_then(|v| v.as_array()) {
            for item in arr {
                let first = item.as_array().and_then(|a| a.first()).unwrap_or(item);
                if let (Some(key), Some(name)) = (
                    first.get("key").and_then(|v| v.as_str()),
                    first.get("name").and_then(|v| v.as_str()),
                ) {
                    attrs.push(UnitAttr {
                        key: key.to_string(),
                        name: name.to_string(),
                    });
                }
            }
        }
        Ok(attrs)
    }

    /// 2. 玩家属性 (`campinfo.json`)
    pub fn get_player_attrs(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Vec<PlayerAttr>> {
        let trigger_map = project.trigger_map_for(map_name)?;
        let mut file_path = trigger_map.map_path.join("campinfo.json");
        if !file_path.is_file() {
            if let Ok(entry) = project.entry_map() {
                let fallback = entry.map_path.join("campinfo.json");
                if fallback.is_file() {
                    file_path = fallback;
                }
            }
        }
        if !file_path.is_file() {
            return Ok(Vec::new());
        }
        let raw: Value = serde_json::from_str(&fs::read_to_string(&file_path)?)?;
        let plain = convert_tuple_format(&raw);
        let mut attrs = Vec::new();

        if let Some(arr) = plain.get("player_res_keys").and_then(|v| v.as_array()) {
            for item in arr {
                let first = item.as_array().and_then(|a| a.first()).unwrap_or(item);
                if let (Some(key), Some(name)) = (
                    first.get("key").and_then(|v| v.as_str()),
                    first.get("name").and_then(|v| v.as_str()),
                ) {
                    let desc = first
                        .get("desc")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    attrs.push(PlayerAttr {
                        key: key.to_string(),
                        name: name.to_string(),
                        desc,
                    });
                }
            }
        }
        Ok(attrs)
    }

    /// 3. 自定义事件 (`customevent.json`)
    pub fn get_custom_events(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Vec<CustomEventItem>> {
        let trigger_map = project.trigger_map_for(map_name)?;
        let file_path = trigger_map.map_path.join("customevent.json");
        if !file_path.is_file() {
            return Ok(Vec::new());
        }
        let raw: Value = serde_json::from_str(&fs::read_to_string(&file_path)?)?;
        let plain = convert_tuple_format(&raw);

        let mut path_map: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        if let Some(group_info) = plain.get("group_info").and_then(|v| v.as_array()) {
            Self::scan_event_groups(group_info, &mut Vec::new(), &mut path_map);
        }

        let mut events = Vec::new();
        if let Some(conf) = plain.get("conf").and_then(|v| v.as_object()) {
            for (_, v) in conf {
                let id = match v.get("id").and_then(|i| i.as_i64()) {
                    Some(i) => i,
                    None => continue,
                };
                let name = v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut args = Vec::new();
                if let Some(arg_list) = v.get("args").and_then(|a| a.as_array()) {
                    for arg_entry in arg_list {
                        let first = arg_entry
                            .as_array()
                            .and_then(|a| a.first())
                            .unwrap_or(arg_entry);
                        let arg_name = first
                            .get("desc")
                            .or_else(|| first.get("name"))
                            .and_then(|s| s.as_str())
                            .unwrap_or("arg")
                            .to_string();
                        let type_code = first.get("type").and_then(|t| t.as_i64()).unwrap_or(0);
                        let lua_type = Self::event_arg_type_to_lua(type_code).to_string();
                        args.push(CustomEventArg {
                            name: arg_name.clone(),
                            lua_type,
                            desc: arg_name,
                        });
                    }
                }
                let path = path_map.get(&id).cloned().unwrap_or_default();
                events.push(CustomEventItem {
                    id,
                    name,
                    path,
                    args,
                });
            }
        }
        events.sort_by_key(|e| e.id);
        Ok(events)
    }

    fn scan_event_groups(
        items: &[Value],
        current_path: &mut Vec<String>,
        path_map: &mut BTreeMap<i64, Vec<String>>,
    ) {
        for item in items {
            if let Some(arr) = item.as_array() {
                if arr.len() >= 3 && arr[0].as_i64() == Some(1) {
                    // 文件夹: [1, folder_name, [children...]]
                    if let (Some(folder_name), Some(children)) =
                        (arr[1].as_str(), arr[2].as_array())
                    {
                        current_path.push(folder_name.to_string());
                        Self::scan_event_groups(children, current_path, path_map);
                        current_path.pop();
                    }
                } else if arr.len() >= 2 && arr[0].as_i64() == Some(2) {
                    // 事件叶子: [2, event_id]
                    if let Some(event_id) = arr[1].as_i64() {
                        path_map.insert(event_id, current_path.clone());
                    }
                }
            }
        }
    }

    fn event_arg_type_to_lua(code: i64) -> &'static str {
        match code {
            100000 => "integer",
            100001 => "boolean",
            100002 => "string",
            100003 => "number",
            100004 => "Point",
            100006 => "Unit",
            100010 => "Item",
            100011 => "Ability",
            100013 => "Player",
            100014 => "Projectile",
            100027 => "UnitGroup",
            100030 => "Destructible",
            100031 => "Particle",
            100032 => "Mover",
            100100 => "UIPrefab",
            _ => "any",
        }
    }

    /// 4. 界面 UI 包 (`ui/*.json`, `ui/prefab/*.json`, `ui/SceneUI.json`)
    pub fn get_ui_package(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<UIPackage> {
        let ui_map = project.ui_map_for(map_name)?;
        let ui_dir = ui_map.map_path.join("ui");
        let mut pkg = UIPackage::default();
        if !ui_dir.is_dir() {
            return Ok(pkg);
        }

        // 读取 ui_config.json 中的画板顺序（若存在）
        let mut canvas_order: Vec<String> = Vec::new();
        let ui_config_path = ui_dir.join("ui_config.json");
        if ui_config_path.is_file() {
            if let Ok(text) = fs::read_to_string(&ui_config_path) {
                if let Ok(val) = serde_json::from_str::<Value>(&text) {
                    if let Some(order) = val.get("ui_order").and_then(|v| v.as_array()) {
                        for item in order {
                            if let Some(s) = item.as_str() {
                                canvas_order.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }

        // 扫描画板与 SceneUI.json
        for entry in fs::read_dir(&ui_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let file_name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if file_name == "ui_config.json" {
                continue;
            }
            if file_name == "SceneUI.json" {
                if let Ok(nodes) = Self::parse_scene_ui_file(&path) {
                    pkg.scene_uis = nodes;
                }
                continue;
            }
            if let Ok(node) = Self::parse_ui_node_file(&path) {
                pkg.canvases.push(node);
            }
        }

        // 按 ui_order 或名称排序画板
        pkg.canvases.sort_by(|a, b| {
            let ia = canvas_order
                .iter()
                .position(|x| x == &a.name || x == &a.uid)
                .unwrap_or(usize::MAX);
            let ib = canvas_order
                .iter()
                .position(|x| x == &b.name || x == &b.uid)
                .unwrap_or(usize::MAX);
            ia.cmp(&ib).then_with(|| a.name.cmp(&b.name))
        });

        // 扫描 prefab 目录
        let prefab_dir = ui_dir.join("prefab");
        if prefab_dir.is_dir() {
            for entry in fs::read_dir(&prefab_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(node) = Self::parse_prefab_file(&path) {
                        pkg.prefabs.push(node);
                    }
                }
            }
            pkg.prefabs.sort_by(|a, b| a.name.cmp(&b.name));
        }

        Ok(pkg)
    }

    fn parse_ui_node_file(path: &Path) -> anyhow::Result<UINode> {
        let text = fs::read_to_string(path)?;
        let val: Value = serde_json::from_str(&text)?;
        let plain = convert_tuple_format(&val);
        Self::value_to_ui_node(&plain)
    }

    fn parse_prefab_file(path: &Path) -> anyhow::Result<UINode> {
        let text = fs::read_to_string(path)?;
        let val: Value = serde_json::from_str(&text)?;
        let plain = convert_tuple_format(&val);
        let name = plain
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let uid = plain
            .get("key")
            .or_else(|| plain.get("uid"))
            .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| Some(v.to_string())))
            .unwrap_or_default();
        let childs = plain
            .get("data")
            .and_then(|d| d.get("children").or_else(|| d.get("childs")))
            .or_else(|| plain.get("children"))
            .or_else(|| plain.get("childs"))
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| Self::value_to_ui_node(item).ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok(UINode {
            name,
            uid,
            node_type: 7,
            childs,
        })
    }

    fn parse_scene_ui_file(path: &Path) -> anyhow::Result<Vec<UINode>> {
        let text = fs::read_to_string(path)?;
        let val: Value = serde_json::from_str(&text)?;
        let plain = convert_tuple_format(&val);
        let mut nodes = Vec::new();
        if let Some(items) = plain.get("items").and_then(|v| v.as_object()) {
            for (k, item) in items {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(k)
                    .to_string();
                let uid = item
                    .get("uid")
                    .and_then(|v| v.as_str())
                    .unwrap_or(k)
                    .to_string();
                nodes.push(UINode {
                    name,
                    uid,
                    node_type: 0,
                    childs: Vec::new(),
                });
            }
        }
        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(nodes)
    }

    fn value_to_ui_node(val: &Value) -> anyhow::Result<UINode> {
        let name = val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unnamed")
            .to_string();
        let uid = val
            .get("uid")
            .or_else(|| val.get("id"))
            .or_else(|| val.get("key"))
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| Some(v.to_string()))
            })
            .unwrap_or_default();
        let node_type = val.get("type").and_then(|v| v.as_i64()).unwrap_or(7);
        let childs = val
            .get("children")
            .or_else(|| val.get("childs"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| Self::value_to_ui_node(c).ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok(UINode {
            name,
            uid,
            node_type,
            childs,
        })
    }

    /// 格式化 UI 画布树文本（与原版 `tcpServer.ts` 中的 `getUICanvas` 输出完全一致）
    pub fn format_ui_canvas_tree(
        pkg: &UIPackage,
        node_path: Option<&str>,
        max_depth: Option<usize>,
    ) -> anyhow::Result<String> {
        if let Some(path_str) = node_path.filter(|s| !s.trim().is_empty()) {
            let segments: Vec<&str> = path_str.split('.').collect();
            let mut target: Option<&UINode> = None;
            for canvas in &pkg.canvases {
                if canvas.name == segments[0] {
                    if segments.len() == 1 {
                        target = Some(canvas);
                    } else {
                        target = Self::find_node_by_path(&canvas.childs, &segments[1..]);
                    }
                    if target.is_some() {
                        break;
                    }
                }
            }
            let node = target.ok_or_else(|| anyhow::anyhow!("Node path not found: {}", path_str))?;
            return Ok(Self::format_node_tree(node, "", true, max_depth, 0));
        }

        if pkg.canvases.is_empty() {
            return Ok("Canvas: (none)".to_string());
        }

        let mut lines = Vec::new();
        for canvas in &pkg.canvases {
            lines.push(format!("Canvas: {}", canvas.name));
            let len = canvas.childs.len();
            for (idx, child) in canvas.childs.iter().enumerate() {
                lines.push(Self::format_node_tree(
                    child,
                    "",
                    idx + 1 == len,
                    max_depth,
                    0,
                ));
            }
            lines.push(String::new());
        }
        Ok(lines.join("\n").trim_end().to_string())
    }

    fn find_node_by_path<'a>(nodes: &'a [UINode], segments: &[&str]) -> Option<&'a UINode> {
        let (&head, rest) = segments.split_first()?;
        let found = nodes.iter().find(|n| n.name == head)?;
        if rest.is_empty() {
            Some(found)
        } else {
            Self::find_node_by_path(&found.childs, rest)
        }
    }

    fn format_node_tree(
        node: &UINode,
        prefix: &str,
        is_last: bool,
        max_depth: Option<usize>,
        current_depth: usize,
    ) -> String {
        let connector = if is_last { "+- " } else { "|- " };
        let type_name = match node.node_type {
            1 => "Button".to_string(),
            3 => "TextLabel".to_string(),
            4 => "Image".to_string(),
            5 => "Progress".to_string(),
            7 => "Layout".to_string(),
            10 => "ScrollView".to_string(),
            18 => "Buff".to_string(),
            27 => "Chat_Box".to_string(),
            38 => "Sequence_Animation".to_string(),
            other => format!("type_{}", other),
        };
        let line = format!(
            "{}{}{} [{}] (uid: {})",
            prefix, connector, node.name, type_name, node.uid
        );

        if let Some(max_d) = max_depth {
            if current_depth >= max_d {
                let child_count = node.childs.len();
                if child_count > 0 {
                    let child_prefix = format!("{}{}", prefix, if is_last { "   " } else { "|  " });
                    return format!("{}\n{}... ({} child nodes)", line, child_prefix, child_count);
                }
                return line;
            }
        }

        let child_prefix = format!("{}{}", prefix, if is_last { "   " } else { "|  " });
        let mut out = vec![line];
        let len = node.childs.len();
        for (i, child) in node.childs.iter().enumerate() {
            out.push(Self::format_node_tree(
                child,
                &child_prefix,
                i + 1 == len,
                max_depth,
                current_depth + 1,
            ));
        }
        out.join("\n")
    }

    /// 5. 时间轴动画 (`uianim.json`)
    pub fn get_ui_anims(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Vec<UIAnimItem>> {
        let ui_map = project.ui_map_for(map_name)?;
        let file_path = ui_map.map_path.join("uianim.json");
        if !file_path.is_file() {
            return Ok(Vec::new());
        }
        let val: Value = serde_json::from_str(&fs::read_to_string(&file_path)?)?;
        let mut items = Vec::new();
        if let Some(anim_map) = val.get("anim").and_then(|v| v.as_object()) {
            for (k, info) in anim_map {
                let name = info
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(k)
                    .to_string();
                let uid = info
                    .get("uid")
                    .and_then(|v| v.as_str())
                    .unwrap_or(k)
                    .to_string();
                let frame = info.get("max_frame").and_then(|v| v.as_i64()).unwrap_or(0);
                let time = (frame as f64) / 30.0;
                let play_mode = info.get("play_mode").and_then(|v| v.as_i64()).unwrap_or(0);
                items.push(UIAnimItem {
                    uid,
                    name,
                    frame,
                    time,
                    play_mode,
                });
            }
        }
        items.sort_by(|a, b| a.uid.cmp(&b.uid));
        Ok(items)
    }

    /// 6. 跳字 (`jumpword.json`)
    pub fn get_jumpwords(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Vec<JumpWordItem>> {
        let ui_map = project.ui_map_for(map_name)?;
        let file_path = ui_map.map_path.join("jumpword.json");
        if !file_path.is_file() {
            return Ok(Vec::new());
        }
        let val: Value = serde_json::from_str(&fs::read_to_string(&file_path)?)?;
        let mut items = Vec::new();
        if let Some(obj) = val.get("items").and_then(|v| v.as_object()) {
            for (k, info) in obj {
                let name = info
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(k)
                    .to_string();
                let uid = info
                    .get("uid")
                    .and_then(|v| v.as_str())
                    .unwrap_or(k)
                    .to_string();
                items.push(JumpWordItem { uid, name });
            }
        }
        items.sort_by(|a, b| a.uid.cmp(&b.uid));
        Ok(items)
    }

    /// 7. 字体 (`font.json`)
    pub fn get_fonts(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Vec<FontItem>> {
        let ui_map = project.ui_map_for(map_name)?;
        let file_path = ui_map.map_path.join("font.json");
        if !file_path.is_file() {
            return Ok(Vec::new());
        }
        let val: Value = serde_json::from_str(&fs::read_to_string(&file_path)?)?;
        let mut items = Vec::new();
        if let Some(obj) = val.as_object() {
            for (k, info) in obj {
                if let Some(name) = info.get("name").and_then(|v| v.as_str()) {
                    items.push(FontItem {
                        uid: k.clone(),
                        name: name.to_string(),
                    });
                }
            }
        }
        items.sort_by(|a, b| a.uid.cmp(&b.uid));
        Ok(items)
    }
}
