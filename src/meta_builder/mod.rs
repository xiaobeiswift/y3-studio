use crate::custom_define::CustomDefine;
use crate::editor_table::table::{EditorTable, TableType};
use crate::env::ProjectContext;
use crate::utils::lua_str::{encode_lua_string, ValidNameGenerator};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaBuildReport {
    pub map_name: String,
    pub generated_files: Vec<PathBuf>,
}

pub struct MetaBuilder;

impl MetaBuilder {
    /// 为项目中所有子地图（以及 `global_script` 若存在）生成全套 Meta 代码
    pub fn build_all_maps(project: &ProjectContext) -> anyhow::Result<Vec<MetaBuildReport>> {
        let mut reports = Vec::new();
        for map_name in project.maps.keys() {
            let report = Self::build_for_map(project, Some(map_name))?;
            reports.push(report);
        }
        Ok(reports)
    }

    /// 为指定子地图（若 `map_name` 为 None 则为当前子地图）生成全套 Meta 代码
    pub fn build_for_map(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<MetaBuildReport> {
        let map = project.resolve_map(map_name)?;
        let mut generated_files = Vec::new();
        let mut active_meta_names = Vec::new();

        // 目标 script 根目录列表：当前子地图的 script 目录 + 若存在 global_script 且当前是主地图则同步输出到 global_script
        let mut target_script_roots = vec![map.script_path.clone()];
        if map.is_entry_map && project.global_script_path.is_dir() {
            target_script_roots.push(project.global_script_path.clone());
        }

        // 1. unitAttrs.lua
        if let Some(content) = Self::make_unit_attrs(project, Some(&map.name))? {
            Self::write_meta_file(
                &target_script_roots,
                "y3-helper/meta/unitAttrs.lua",
                &content,
                &mut generated_files,
            )?;
            active_meta_names.push("y3-helper.meta.unitAttrs");
        } else {
            Self::remove_meta_file(&target_script_roots, "y3-helper/meta/unitAttrs.lua");
        }

        // 2. playerAttrs.lua
        if let Some(content) = Self::make_player_attrs(project, Some(&map.name))? {
            Self::write_meta_file(
                &target_script_roots,
                "y3-helper/meta/playerAttrs.lua",
                &content,
                &mut generated_files,
            )?;
            active_meta_names.push("y3-helper.meta.playerAttrs");
        } else {
            Self::remove_meta_file(&target_script_roots, "y3-helper/meta/playerAttrs.lua");
        }

        // 3. customEvents.lua
        if let Some(content) = Self::make_custom_events(project, Some(&map.name))? {
            Self::write_meta_file(
                &target_script_roots,
                "y3-helper/meta/customEvents.lua",
                &content,
                &mut generated_files,
            )?;
            active_meta_names.push("y3-helper.meta.customEvents");
        } else {
            Self::remove_meta_file(&target_script_roots, "y3-helper/meta/customEvents.lua");
        }

        // 4. ui.lua
        if let Some(content) = Self::make_ui(project, Some(&map.name))? {
            Self::write_meta_file(
                &target_script_roots,
                "y3-helper/meta/ui.lua",
                &content,
                &mut generated_files,
            )?;
            active_meta_names.push("y3-helper.meta.ui");
        } else {
            Self::remove_meta_file(&target_script_roots, "y3-helper/meta/ui.lua");
        }

        // 5. uiAnim.lua
        if let Some(content) = Self::make_ui_anim(project, Some(&map.name))? {
            Self::write_meta_file(
                &target_script_roots,
                "y3-helper/meta/uiAnim.lua",
                &content,
                &mut generated_files,
            )?;
            active_meta_names.push("y3-helper.meta.uiAnim");
        } else {
            Self::remove_meta_file(&target_script_roots, "y3-helper/meta/uiAnim.lua");
        }

        // 6. jumpword.lua
        if let Some(content) = Self::make_jumpword(project, Some(&map.name))? {
            Self::write_meta_file(
                &target_script_roots,
                "y3-helper/meta/jumpword.lua",
                &content,
                &mut generated_files,
            )?;
            active_meta_names.push("y3-helper.meta.jumpword");
        } else {
            Self::remove_meta_file(&target_script_roots, "y3-helper/meta/jumpword.lua");
        }

        // 7. font.lua
        if let Some(content) = Self::make_font(project, Some(&map.name))? {
            Self::write_meta_file(
                &target_script_roots,
                "y3-helper/meta/font.lua",
                &content,
                &mut generated_files,
            )?;
            active_meta_names.push("y3-helper.meta.font");
        } else {
            Self::remove_meta_file(&target_script_roots, "y3-helper/meta/font.lua");
        }

        // 8. objects.lua (仅类型注解，不加入运行时 init.lua include，与原版 metaBuilder/index.ts 一致)
        if let Some(content) = Self::make_objects(project, Some(&map.name))? {
            Self::write_meta_file(
                &target_script_roots,
                "y3-helper/meta/objects.lua",
                &content,
                &mut generated_files,
            )?;
        }

        // 9. init.lua
        let init_content = active_meta_names
            .iter()
            .map(|m| format!("include '{}'", m))
            .collect::<Vec<_>>()
            .join("\r\n");
        Self::write_meta_file(
            &target_script_roots,
            "y3-helper/meta/init.lua",
            &format!("{}\r\n", init_content),
            &mut generated_files,
        )?;

        // 10. 若已初始化插件目录，同步生成 y3-helper/plugin/map-declare.d.ts
        for root in &target_script_roots {
            let plugin_dir = root.join("y3-helper/plugin");
            if plugin_dir.is_dir() {
                if let Ok(ts_content) = Self::make_ts_meta(project, Some(&map.name)) {
                    let ts_path = plugin_dir.join("map-declare.d.ts");
                    fs::write(&ts_path, ts_content)?;
                    generated_files.push(ts_path);
                }
            }
        }

        Ok(MetaBuildReport {
            map_name: map.name.clone(),
            generated_files,
        })
    }

    fn write_meta_file(
        roots: &[PathBuf],
        rel_path: &str,
        content: &str,
        out_list: &mut Vec<PathBuf>,
    ) -> anyhow::Result<()> {
        for root in roots {
            let full = root.join(rel_path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full, content)?;
            out_list.push(full);
        }
        Ok(())
    }

    fn remove_meta_file(roots: &[PathBuf], rel_path: &str) {
        for root in roots {
            let full: PathBuf = root.join(rel_path);
            if full.is_file() {
                let _ = fs::remove_file(full);
            }
        }
    }

    pub fn make_unit_attrs(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let attrs = CustomDefine::get_unit_attrs(project, map_name)?;
        if attrs.is_empty() {
            return Ok(None);
        }
        let attr_enums = attrs
            .iter()
            .map(|a| format!("y3.const.UnitAttr[\"{}\"] = \"{}\"", a.name, a.key))
            .collect::<Vec<_>>()
            .join("\r\n");
        let meta_enums = attrs
            .iter()
            .map(|a| format!("    [\"{}\"] = \"{}\",", a.name, a.key))
            .collect::<Vec<_>>()
            .join("\r\n");

        Ok(Some(format!(
            "y3.const.UnitAttr = y3.const.UnitAttr or {{}}\r\n\r\n{}\r\n\r\n---@enum(key, partial) y3.Const.UnitAttr\r\n---@diagnostic disable-next-line: inject-field\r\ny3.const.CustomUnitAttr = {{\r\n{}\r\n}}\r\n",
            attr_enums, meta_enums
        )))
    }

    pub fn make_player_attrs(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let attrs = CustomDefine::get_player_attrs(project, map_name)?;
        if attrs.is_empty() {
            return Ok(None);
        }
        let attr_enums = attrs
            .iter()
            .map(|a| format!("y3.const.PlayerAttr[\"{}\"] = \"{}\"", a.name, a.key))
            .collect::<Vec<_>>()
            .join("\r\n");
        let meta_enums = attrs
            .iter()
            .map(|a| format!("    [\"{}\"] = \"{}\",", a.name, a.key))
            .collect::<Vec<_>>()
            .join("\r\n");

        Ok(Some(format!(
            "y3.const.PlayerAttr = y3.const.PlayerAttr or {{}}\r\n\r\n{}\r\n\r\n---@enum(key, partial) y3.Const.PlayerAttr\r\n---@diagnostic disable-next-line: inject-field\r\ny3.const.CustomPlayerAttr = {{\r\n{}\r\n}}\r\n",
            attr_enums, meta_enums
        )))
    }

    pub fn make_custom_events(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let events = CustomDefine::get_custom_events(project, map_name)?;
        if events.is_empty() {
            return Ok(None);
        }
        let mut name_gen = ValidNameGenerator::new();
        let event_category = "游戏-消息";

        let fields = events
            .iter()
            .map(|ev| {
                let mut arg_gen = ValidNameGenerator::new();
                let args = ev
                    .args
                    .iter()
                    .map(|a| format!(", {}: {}", arg_gen.get_valid_name(&a.name, None), a.lua_type))
                    .collect::<String>();
                format!("---@field call fun(name: '{}'{} )", ev.name, args).replace(" )", ")")
            })
            .collect::<Vec<_>>()
            .join("\r\n");

        let impls = events
            .iter()
            .map(|ev| {
                let mut arg_gen = ValidNameGenerator::new();
                let args = ev
                    .args
                    .iter()
                    .map(|a| format!(", {}", arg_gen.get_valid_name(&a.name, None)))
                    .collect::<String>();
                let mut body_arg_gen = ValidNameGenerator::new();
                let body_lines = ev
                    .args
                    .iter()
                    .map(|a| {
                        let valid = body_arg_gen.get_valid_name(&a.name, None);
                        if Self::is_basic_lua_type(&a.lua_type) {
                            format!("        [{}] = {}", encode_lua_string(&a.name), valid)
                        } else {
                            format!(
                                "        [{}] = y3.py_converter.lua_to_py_by_lua_type('{}', {})",
                                encode_lua_string(&a.name),
                                a.lua_type,
                                valid
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",\r\n");
                format!(
                    "y3.eca.register_custom_event_impl('{}', function (_{})\r\n    y3.game.send_custom_event({}, {{\r\n{}\r\n    }})\r\nend)",
                    ev.name, args, ev.id, body_lines
                )
            })
            .collect::<Vec<_>>()
            .join("\r\n\r\n");

        let kv = events
            .iter()
            .map(|ev| format!("y3.const.CustomEventName['{}'] = {}", ev.name, ev.id))
            .collect::<Vec<_>>()
            .join("\r\n");

        let table_fields = events
            .iter()
            .map(|ev| format!("    ['{}'] = {},", ev.name, ev.id))
            .collect::<Vec<_>>()
            .join("\r\n");

        let resolves = events
            .iter()
            .map(|ev| {
                let encoded_name = encode_lua_string(&ev.name);
                let data_lines = ev
                    .args
                    .iter()
                    .map(|a| {
                        let enc_arg = encode_lua_string(&a.name);
                        if Self::is_basic_lua_type(&a.lua_type) {
                            format!("        [{}] = data.c_param_dict[{}],", enc_arg, enc_arg)
                        } else {
                            format!(
                                "        [{}] = y3.py_converter.py_to_lua_by_lua_type('{}', data.c_param_dict[{}]),",
                                enc_arg, a.lua_type, enc_arg
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\r\n");
                format!(
                    "y3.eca.register_custom_event_resolve({}, function (data)\r\n    data.name = {}\r\n    data.data = {{\r\n{}\r\n    }}\r\n    return data\r\nend)",
                    encoded_name, encoded_name, data_lines
                )
            })
            .collect::<Vec<_>>()
            .join("\r\n");

        let mut alias_lines = Vec::new();
        let mut class_fields = Vec::new();
        for ev in &events {
            let valid_ev_name = name_gen.get_valid_name(&ev.name, None);
            let encoded_name = encode_lua_string(&ev.name);
            let data_fields = ev
                .args
                .iter()
                .map(|a| format!("[{}]: {}", encode_lua_string(&a.name), a.lua_type))
                .collect::<Vec<_>>()
                .join(", ");
            alias_lines.push(format!(
                "---@alias EventParam.{}.{} {{ c_param_1: {}, c_param_dict: py.Dict, event: {}, data: {{ {} }} }}",
                event_category, valid_ev_name, ev.id, encoded_name, data_fields
            ));
            class_fields.push(format!(
                "---@diagnostic disable-next-line: duplicate-doc-field\r\n---@field event fun(self: Game, event: \"{}\", event_id: {}, callback: fun(trigger: Trigger, data: EventParam.{}.{}))",
                event_category, encoded_name, event_category, valid_ev_name
            ));
        }

        Ok(Some(format!(
            "---@class ECAHelper\r\n{}\r\n\r\n---@diagnostic disable: invisible\r\n\r\ny3.eca = y3.eca or {{}}\r\ny3.eca.register_custom_event_impl = y3.eca.register_custom_event_impl or function (name, impl) end\r\ny3.eca.register_custom_event_resolve = y3.eca.register_custom_event_resolve or function (name, resolve) end\r\n\r\n{}\r\n\r\ny3.const.CustomEventName = y3.const.CustomEventName or {{}}\r\n\r\n{}\r\n\r\n---@enum(key, partial) y3.Const.CustomEventName\r\nlocal CustomEventName = {{\r\n{}\r\n}}\r\n\r\n{}\r\n\r\n{}\r\n\r\n---@class Game\r\n{}\r\n",
            fields,
            impls,
            kv,
            table_fields,
            resolves,
            alias_lines.join("\r\n"),
            class_fields.join("\r\n")
        )))
    }

    fn is_basic_lua_type(t: &str) -> bool {
        matches!(t, "boolean" | "number" | "integer" | "string" | "table")
    }

    pub fn make_ui(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let pkg = CustomDefine::get_ui_package(project, map_name)?;
        if pkg.scene_uis.is_empty() {
            return Ok(None);
        }
        let ui_enums = pkg
            .scene_uis
            .iter()
            .map(|s| format!("y3.const.SceneUI[\"{}\"] = \"{}\"", s.name, s.uid))
            .collect::<Vec<_>>()
            .join("\r\n");
        let meta_enums = pkg
            .scene_uis
            .iter()
            .map(|s| format!("    [\"{}\"] = \"{}\",", s.name, s.uid))
            .collect::<Vec<_>>()
            .join("\r\n");

        Ok(Some(format!(
            "y3.const.SceneUI = y3.const.SceneUI or {{}}\r\n\r\n{}\r\n\r\n---@enum(key, partial) y3.Const.SceneUI\r\nlocal SceneUI = {{\r\n{}\r\n}}\r\n",
            ui_enums, meta_enums
        )))
    }

    pub fn make_ui_anim(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let anims = CustomDefine::get_ui_anims(project, map_name)?;
        if anims.is_empty() {
            return Ok(None);
        }
        let keys = anims
            .iter()
            .map(|a| format!("y3.const.UIAnimKey[\"{}\"] = \"{}\"", a.name, a.uid))
            .collect::<Vec<_>>()
            .join("\r\n");
        let meta_keys = anims
            .iter()
            .map(|a| format!("    [\"{}\"] = \"{}\",", a.name, a.uid))
            .collect::<Vec<_>>()
            .join("\r\n");

        Ok(Some(format!(
            "y3.const.UIAnimKey = y3.const.UIAnimKey or {{}}\r\n\r\n{}\r\n\r\n---@enum(key, partial) y3.Const.UIAnimKey\r\nlocal UIAnimKey = {{\r\n{}\r\n}}\r\n",
            keys, meta_keys
        )))
    }

    pub fn make_jumpword(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let words = CustomDefine::get_jumpwords(project, map_name)?;
        if words.is_empty() {
            return Ok(None);
        }
        let jw = words
            .iter()
            .map(|w| format!("y3.const.FloatTextJumpType[\"{}\"] = {}", w.name, w.uid))
            .collect::<Vec<_>>()
            .join("\r\n");
        let meta_jw = words
            .iter()
            .map(|w| format!("    [\"{}\"] = {},", w.name, w.uid))
            .collect::<Vec<_>>()
            .join("\r\n");

        Ok(Some(format!(
            "y3.const.FloatTextJumpType = y3.const.FloatTextJumpType or {{}}\r\n\r\n{}\r\n\r\n---@enum(key, partial) y3.Const.FloatTextJumpType\r\nlocal FloatTextJumpType = {{\r\n{}\r\n}}\r\n",
            jw, meta_jw
        )))
    }

    pub fn make_font(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let fonts = CustomDefine::get_fonts(project, map_name)?;
        if fonts.is_empty() {
            return Ok(None);
        }
        let f = fonts
            .iter()
            .map(|item| format!("y3.const.FloatTextType[\"{}\"] = \"{}\"", item.name, item.uid))
            .collect::<Vec<_>>()
            .join("\r\n");
        let meta_f = fonts
            .iter()
            .map(|item| format!("    [\"{}\"] = \"{}\",", item.name, item.uid))
            .collect::<Vec<_>>()
            .join("\r\n");

        Ok(Some(format!(
            "y3.const.FloatTextType = y3.const.FloatTextType or {{}}\r\n\r\n{}\r\n\r\n---@enum(key, partial) y3.Const.FloatTextType\r\nlocal FloatTextType = {{\r\n{}\r\n}}\r\n",
            f, meta_f
        )))
    }

    pub fn make_objects(
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let mut sections = Vec::new();
        for &tt in TableType::ALL {
            let table = EditorTable::open(project, map_name, tt)?;
            let keys = table.list_keys()?;
            let en = tt.short_en();
            let mut chars = en.chars();
            let pascal = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            };
            let key_lines = keys
                .iter()
                .map(|k| format!("---| {}", k))
                .collect::<Vec<_>>()
                .join("\r\n");
            sections.push(format!("---@alias py.{}KeyEnum integer\r\n{}\r\n", pascal, key_lines));
        }
        Ok(Some(sections.join("\r\n")))
    }

    pub fn make_ts_meta(project: &ProjectContext, map_name: Option<&str>) -> anyhow::Result<String> {
        let attrs = CustomDefine::get_unit_attrs(project, map_name)?;
        let lines = attrs
            .iter()
            .map(|a| format!("    {}: number,", serde_json::to_string(&a.name).unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\r\n");
        Ok(format!(
            "\r\ndeclare interface MapUnit {{\r\n{}\r\n}}\r\n",
            lines
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_builder_full() {
        let tmp = tempfile::tempdir().unwrap();
        let proj_dir = tmp.path().join("Proj");
        let map_dir = proj_dir.join("maps/EntryMap");
        fs::create_dir_all(map_dir.join("script/y3-helper/plugin")).unwrap();
        fs::write(
            proj_dir.join("header.project"),
            r#"{"entry_map": {"id": 1}}"#,
        )
        .unwrap();
        fs::write(map_dir.join("header.map"), r#"{"id": 1}"#).unwrap();
        fs::write(
            map_dir.join("attr.json"),
            r#"{"user_specified": [[{"key": "crit_damage", "name": "暴击伤害"}]]}"#,
        )
        .unwrap();
        fs::write(
            map_dir.join("campinfo.json"),
            r#"{"player_res_keys": [[{"key": "wood", "name": "木材", "desc": "基础资源"}]]}"#,
        )
        .unwrap();

        let ctx = ProjectContext::load_from_path(&proj_dir).unwrap();
        let report = MetaBuilder::build_for_map(&ctx, None).unwrap();
        assert!(!report.generated_files.is_empty());

        let unit_attrs_lua =
            fs::read_to_string(map_dir.join("script/y3-helper/meta/unitAttrs.lua")).unwrap();
        assert!(unit_attrs_lua.contains("y3.const.UnitAttr[\"暴击伤害\"] = \"crit_damage\""));

        let init_lua = fs::read_to_string(map_dir.join("script/y3-helper/meta/init.lua")).unwrap();
        assert!(init_lua.contains("include 'y3-helper.meta.unitAttrs'"));
        assert!(init_lua.contains("include 'y3-helper.meta.playerAttrs'"));
    }
}
