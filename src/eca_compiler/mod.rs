use crate::custom_define::CustomDefine;
use crate::editor_table::excel::ExcelSheet;
use crate::editor_table::table::{EditorTable, TableType};
use crate::env::ProjectContext;
use crate::utils::lua_str::{encode_lua_string, encode_lua_value};
use crate::utils::y3json::convert_tuple_format;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const EVENT_META_JSON: &str =
    include_str!("../../y3-helper-vscode/src/helper_meta/trigger/event.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcaCompileReport {
    pub map_name: String,
    pub trigger_count: usize,
    pub function_count: usize,
    pub object_trigger_count: usize,
    pub written_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum RuleKind {
    Template {
        tpl: String,
        arg_names: Option<Vec<String>>,
    },
    Dict(HashMap<String, String>),
    Null,
}

pub struct EcaFormatter {
    pub rules: HashMap<String, RuleKind>,
    pub event_names: HashMap<String, String>,
    pub func_names: HashMap<String, String>,
}

impl EcaFormatter {
    pub fn new(
        project: &ProjectContext,
        map_name: Option<&str>,
        eca_override_dir: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let mut fmt = Self {
            rules: HashMap::new(),
            event_names: HashMap::new(),
            func_names: HashMap::new(),
        };
        fmt.fill_static();
        fmt.fill_events()?;
        fmt.fill_map_defined(project, map_name)?;
        if let Some(override_dir) = eca_override_dir {
            let _ = fmt.load_excel_override(override_dir);
        }
        Ok(fmt)
    }

    pub fn set_tpl(&mut self, key: &str, tpl: &str, arg_names: Option<Vec<&str>>) {
        self.rules.insert(
            key.to_string(),
            RuleKind::Template {
                tpl: tpl.to_string(),
                arg_names: arg_names.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            },
        );
    }

    pub fn set_dict(&mut self, key: &str, map: HashMap<String, String>) {
        self.rules.insert(key.to_string(), RuleKind::Dict(map));
    }

    fn fill_events(&mut self) -> anyhow::Result<()> {
        let parsed: Value = serde_json::from_str(EVENT_META_JSON)?;
        if let Some(obj) = parsed.as_object() {
            for (k, v) in obj {
                if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                    self.event_names.insert(k.clone(), name.to_string());
                }
            }
        }
        Ok(())
    }

    fn fill_map_defined(
        &mut self,
        project: &ProjectContext,
        map_name: Option<&str>,
    ) -> anyhow::Result<()> {
        // 100238: 自定义事件
        let mut ev_map = HashMap::new();
        for ev in CustomDefine::get_custom_events(project, map_name).unwrap_or_default() {
            ev_map.insert(ev.id.to_string(), encode_lua_string(&ev.name));
        }
        self.set_dict("100238", ev_map);

        // 100259: 字体类型
        let mut font_map = HashMap::new();
        let default_fonts = [
            ("physics", "物理伤害"),
            ("magic", "魔法伤害"),
            ("real", "真实伤害"),
            ("heal", "治疗"),
            ("get_gold", "获取金币"),
            ("0", "系统字体"),
            ("MSYH", "微软雅黑"),
            ("HKHeiW9", "华康黑体W9"),
            ("HKHeiW12", "华康黑体W12"),
            ("HKSongW9", "华康标题宋W9"),
            ("HKWeiBeiW7", "华康魏碑W7"),
            ("HKXinZongYiW7", "华康新综艺体W7"),
            ("HKXinZongYiW9", "华康新综艺体W9"),
            ("HKYuanW5", "华康圆体W5"),
            ("HKYuanW7", "华康圆体W7"),
            ("HKYuanW9", "华康圆体W9"),
        ];
        for (k, v) in default_fonts {
            font_map.insert(k.to_string(), encode_lua_string(v));
        }
        for f in CustomDefine::get_fonts(project, map_name).unwrap_or_default() {
            font_map.insert(f.uid, encode_lua_string(&f.name));
        }
        self.set_dict("100259", font_map);

        // 100333: 跳字类型
        let mut jw_map = HashMap::new();
        for w in CustomDefine::get_jumpwords(project, map_name).unwrap_or_default() {
            jw_map.insert(w.uid, encode_lua_string(&w.name));
        }
        self.set_dict("100333", jw_map);
        Ok(())
    }

    fn fill_static(&mut self) {
        self.set_tpl("100006", "y3.unit.get_by_res_id({})", None);
        self.set_dict(
            "100015",
            HashMap::from([
                ("==".into(), "==".into()),
                ("!=".into(), "~=".into()),
                (">".into(), ">".into()),
                ("<".into(), "<".into()),
                (">=".into(), ">=".into()),
                ("<=".into(), "<=".into()),
            ]),
        );
        self.set_dict(
            "100016",
            HashMap::from([
                ("+".into(), "+".into()),
                ("-".into(), "-".into()),
                ("*".into(), "*".into()),
                ("/".into(), "/".into()),
                ("%".into(), "%".into()),
            ]),
        );
        self.set_tpl("100025", "y3.player({})", None);
        self.set_dict(
            "100035",
            HashMap::from([("==".into(), "==".into()), ("!=".into(), "~=".into())]),
        );
        self.set_dict(
            "100064",
            HashMap::from([
                ("0".into(), encode_lua_string("物理")),
                ("1".into(), encode_lua_string("法术")),
                ("2".into(), encode_lua_string("真实")),
            ]),
        );
        self.set_dict(
            "100175",
            HashMap::from([
                ("1".into(), "error".into()),
                ("2".into(), "warn".into()),
                ("3".into(), "info".into()),
            ]),
        );
        self.set_tpl("100205", "y3.destructible.get_by_id({})", None);
        self.set_tpl("PLAYER", "y3.player({})", None);
        self.set_tpl("POINT", "y3.point(0, 0)", None);
        self.set_tpl("PLAYER_GROUP", "y3.player_group.create()", None);
        self.set_tpl("UNIT_GROUP", "y3.unit_group.create()", None);
        self.set_tpl("TABLE", "%{%}", None);
        self.set_tpl("UNIT_ENTITY_POINT", "{}:get_point()", None);
        self.set_tpl("ALL_PLAYER", "y3.player_group.get_all_players()", None);
        self.set_tpl(
            "CREATE_HARM_TEXT_NEW",
            "y3.ui.create_floating_text2({point}, {type}, {str}, {jump_word_track}, {player_group})",
            Some(vec![
                "point",
                "type",
                "str",
                "player_group",
                "jump_word_track",
            ]),
        );
        self.set_tpl("SET_DEST_IS_SELECTED", "{}:set_can_be_selected({})", None);
        self.set_tpl(
            "APPLY_DAMAGE",
            "{unit}:damage {\n    target  = {target},\n    damage  = {damage},\n    type    = {type},\n  <?socket  = {socket},?>\n  <?ability = {ability},?>\n  <?no_miss = {no_miss},?>\n  <?critical      = {critical},?>\n  <?particle      = {particle},?>\n  <?text_type     = {text_enable} and {text_type},?>\n  <?text_track    = {text_track},?>\n  <?common_attack = {common_attack},?>\n  <?attack_type   = {attack_type},?>\n  <?pos_socket    = {pos_socket},?>\n}",
            Some(vec![
                "unit",
                "ability",
                "target",
                "type",
                "damage",
                "text_enable",
                "common_attack",
                "critical",
                "no_miss",
                "particle",
                "socket",
                "text_type",
                "text_track",
                "attack_type",
                "pos_socket",
            ]),
        );
        self.set_tpl(
            "RUN_LOOP_TIMER_NO_SAVE",
            "y3.timer.loop({1}, function (timer)\n    {3}\nend, nil, {2})",
            None,
        );
        self.rules.insert("NONE_ABILITY".to_string(), RuleKind::Null);
        self.set_tpl("DISPLAY_INFO_TO_PLAYER", "{}:display_info({}, {})", None);
        self.set_tpl("ANY_VAR_TO_STR", "tostring({})", None);
        self.set_tpl(
            "CHANGE_MODEL_TEXTURE",
            "{}:change_model_texture({}, {}, {}, {})",
            None,
        );
        self.set_tpl("GET_CUS_EVENT_PARAM", "params.data[{}]", None);
        self.set_tpl("KILL_UNIT", "{}:kill_by({})", None);
        self.set_tpl("KILLER_UNIT", "params.source_unit", None);
        self.set_tpl("KILLED_UNIT", "params.target_unit", None);
        self.set_tpl(
            "EVALUATE_LUA_CODE",
            "y3.rt.evaluate({}, {}, {}, {}, {}, {})",
            None,
        );
        self.set_tpl("SET_VARIABLE", "{} = {}", None);
        self.set_tpl("INT_PLUS_ONE", "({} + 1)", None);
        self.set_tpl("STRING_COMPARE", "({} {} {})", None);
        self.set_tpl("BOOL_COMPARE", "({} {} {})", None);
        self.set_tpl("TRIGGER_PLAYER", "params.player", None);
        self.set_tpl("GET_BOOLEAN_KV", "{}:kv_load({}, \"boolean\")", None);
        self.set_tpl("EXTRACT_STR", "string.sub({}, {} + 1, {})", None);
        self.set_tpl("VARIABLE", "{}", None);
        self.set_tpl("HAS_KV_ANY", "{}:kv_has({})", None);
        self.set_tpl("$100002", "{}[{}]", None);
        self.set_tpl("$100009", "{}", None);
        self.set_tpl("$100028", "{}[{}]", None);
        self.set_tpl("$100148", "{}[{}]", None);
        self.set_tpl("PRINT_MESSAGE_ACTION_TO_DIALOG", "log.{}({})", None);
        self.set_tpl("GET_PLAYER_KV", "{}:kv_load({}, \"Player\")", None);
        self.set_tpl("SET_ENTITY_KV", "{}:kv_save({}, {})", None);
        self.set_tpl("GET_INTEGER_KV", "{}:kv_load({}, \"integer\")", None);
        self.set_tpl(
            "RUN_ONCE_TIMER_BY_FRAME_NO_SAVE",
            "y3.ltimer.wait_frame({}, function (timer)\n    {}\nend)",
            None,
        );
        self.set_tpl(
            "RUN_ONCE_TIMER_NO_SAVE",
            "y3.timer.wait({}, function (timer)\n    {}\nend)",
            None,
        );
        self.set_tpl("MATH_OPERATION", "{} {} {}", None);
        self.set_tpl(
            "FOR_INT_DO_ACTION",
            "for i = {2}, {3} do\n    {1} = i\n    {4}\n    ::continue::\nend",
            None,
        );
        self.set_tpl(
            "FOR_INT_DO_ACTION_NEW",
            "for i = {2}, {3}<?, {5}?> do\n    {1} = i\n    {4}\n    ::continue::\nend",
            None,
        );
    }

    pub fn load_excel_override(&mut self, dir: &Path) -> anyhow::Result<()> {
        let mut target_dir = dir.join("excel/编辑器/触发器");
        if !target_dir.is_dir() {
            target_dir = dir.to_path_buf();
        }
        if !target_dir.is_dir() {
            return Ok(());
        }
        let re_req = Regex::new(r"\{#(\d+)\}")?;
        let re_opt = Regex::new(r"<\$(\d+)>")?;

        for fname in [
            "02触发器事件条件表.xlsx",
            "03触发器动作表.xlsx",
            "04触发器函数表.xlsx",
        ] {
            let fpath = target_dir.join(fname);
            if !fpath.is_file() {
                continue;
            }
            if let Ok(sheet) = ExcelSheet::load_file(&fpath, Some("触发器")) {
                let table = sheet.make_table(Some((2, 1)), 1);
                for (key, row) in table {
                    let lua_text = match row.get("lua文本") {
                        Some(s) if !s.is_empty() => s.clone(),
                        _ => continue,
                    };
                    let params: Vec<&str> = row
                        .get("参数")
                        .map(|s| s.split(';').collect())
                        .unwrap_or_default();
                    let optionals: Vec<&str> = row
                        .get("可选参数")
                        .map(|s| s.split(';').collect())
                        .unwrap_or_default();

                    let step1 = re_req
                        .replace_all(&lua_text, |caps: &regex::Captures| {
                            let idx: usize = caps[1].parse().unwrap_or(0);
                            let mut s = format!("{{{}}}", idx + 1);
                            if params.get(idx) == Some(&"ACTION_LIST") {
                                s = format!("\n{}\n", s);
                            }
                            s
                        })
                        .to_string();

                    let step2 = re_opt
                        .replace_all(&step1, |caps: &regex::Captures| {
                            let idx: usize = caps[1].parse().unwrap_or(0);
                            let mut s = format!("{{{}}}", idx + 1 + params.len());
                            if optionals.get(idx) == Some(&"ACTION_LIST") {
                                s = format!("\n{}\n", s);
                            }
                            s
                        })
                        .to_string();

                    self.set_tpl(&key, &step2, None);
                }
            }
        }
        Ok(())
    }

    pub fn apply_template(
        &self,
        tpl: &str,
        arg_names: Option<&[String]>,
        args: &[Option<String>],
    ) -> String {
        let mut named = HashMap::new();
        if let Some(names) = arg_names {
            for (i, n) in names.iter().enumerate() {
                if let Some(Some(v)) = args.get(i) {
                    named.insert(n.clone(), v.clone());
                }
            }
        }

        // 先处理可选段 `<? ... ?>`
        let opt_re = Regex::new(r"(?s)<\?(.*?)\?>").unwrap();
        let placeholder_re = Regex::new(r"\{([a-zA-Z0-9_]*)\}").unwrap();

        let mut seq_idx = 0usize;
        let after_opt = opt_re
            .replace_all(tpl, |caps: &regex::Captures| {
                let inner = &caps[1];
                let mut missing = false;
                let replaced = placeholder_re.replace_all(inner, |pcaps: &regex::Captures| {
                    let key = &pcaps[1];
                    let val = if key.is_empty() {
                        let v = args.get(seq_idx).and_then(|x| x.clone());
                        seq_idx += 1;
                        v
                    } else if let Ok(num) = key.parse::<usize>() {
                        args.get(num.saturating_sub(1)).and_then(|x| x.clone())
                    } else {
                        named.get(key).cloned()
                    };
                    match val {
                        Some(v) if v != "nil" && !v.is_empty() => v,
                        _ => {
                            missing = true;
                            String::new()
                        }
                    }
                });
                if missing {
                    String::new()
                } else {
                    replaced.into_owned()
                }
            })
            .to_string();

        seq_idx = 0;
        let final_str = placeholder_re
            .replace_all(&after_opt, |pcaps: &regex::Captures| {
                let key = &pcaps[1];
                let val = if key.is_empty() {
                    let v = args.get(seq_idx).and_then(|x| x.clone());
                    seq_idx += 1;
                    v
                } else if let Ok(num) = key.parse::<usize>() {
                    args.get(num.saturating_sub(1)).and_then(|x| x.clone())
                } else {
                    named.get(key).cloned()
                };
                val.unwrap_or_else(|| "nil".to_string())
            })
            .to_string();

        final_str.replace("%{", "{").replace("%}", "}")
    }

    /// 格式化整个 Lua 文件的缩进（与原版 `Formatter.asFileContent` 完全一致）
    pub fn as_file_content(&self, content: &str) -> String {
        let mut indent: usize = 0;
        let mut lines_out = Vec::new();

        for raw_line in content.lines() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let dec_before = trimmed == "end"
                || trimmed.starts_with("end,")
                || trimmed.starts_with("end)")
                || trimmed == "else"
                || trimmed.starts_with("elseif ")
                || trimmed == "}"
                || trimmed.starts_with("},");

            if dec_before && indent > 0 {
                indent -= 1;
            }

            lines_out.push(format!("{}{}", "    ".repeat(indent), trimmed));

            let inc_after = trimmed.ends_with(" then")
                || trimmed.ends_with(" do")
                || trimmed == "else"
                || trimmed.ends_with('{')
                || (trimmed.contains("function") && trimmed.ends_with(')'));

            if inc_after {
                indent += 1;
            }
        }
        lines_out.join("\r\n") + "\r\n"
    }
}

pub struct EcaCompiler;

impl EcaCompiler {
    /// 编译指定项目、指定子地图的所有 ECA 触发器、ECA 函数、全局变量与物编触发器到 `{map}/script/y3-trigger/`
    pub fn compile_map(
        project: &ProjectContext,
        map_name: Option<&str>,
        eca_override_dir: Option<&Path>,
    ) -> anyhow::Result<EcaCompileReport> {
        let out_map = project.resolve_map(map_name)?;
        let in_map = project.trigger_map_for(map_name)?;
        let mut formatter = EcaFormatter::new(project, map_name, eca_override_dir)?;

        let mut include_files: Vec<String> = Vec::new();
        let mut write_cache: BTreeMap<String, String> = BTreeMap::new();

        // 1. 编译全局变量 globaltriggervariable.json
        let global_var_path = in_map.map_path.join("globaltriggervariable.json");
        if global_var_path.is_file() {
            if let Ok(text) = fs::read_to_string(&global_var_path) {
                let code = Self::compile_global_variables(&text, &formatter)?;
                if !code.trim().is_empty() {
                    let inc = "y3-trigger/全局变量.lua".to_string();
                    include_files.push(inc.clone());
                    write_cache.insert(inc, formatter.as_file_content(&code));
                }
            }
        }

        // 2. 先扫描函数以预注册函数 ID -> 函数名
        let func_dir = in_map.map_path.join("global_trigger/function");
        let func_files = Self::scan_json_files(&func_dir);
        for (_, rel_name, content) in &func_files {
            if let Ok(val) = serde_json::from_str::<Value>(content) {
                let plain = convert_tuple_format(&val);
                if let (Some(id), Some(name)) = (
                    plain
                        .get("key")
                        .or_else(|| plain.get("uid"))
                        .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| Some(v.to_string()))),
                    plain.get("name").and_then(|v| v.as_str()),
                ) {
                    formatter.func_names.insert(id, name.to_string());
                } else {
                    let _ = rel_name;
                }
            }
        }

        // 3. 编译触发器 global_trigger/trigger
        let trig_dir = in_map.map_path.join("global_trigger/trigger");
        let trig_files = Self::scan_json_files(&trig_dir);
        let mut trigger_count = 0usize;
        for (_, rel_name, content) in &trig_files {
            if let Ok(code) = Self::compile_eca_json(content, false, &formatter) {
                let inc = format!("y3-trigger/trigger/{}.lua", rel_name);
                include_files.push(inc.clone());
                write_cache.insert(inc, formatter.as_file_content(&code));
                trigger_count += 1;
            }
        }

        // 4. 编译函数 global_trigger/function
        let mut function_count = 0usize;
        for (_, rel_name, content) in &func_files {
            if let Ok(code) = Self::compile_eca_json(content, true, &formatter) {
                let inc = format!("y3-trigger/function/{}.lua", rel_name);
                include_files.push(inc.clone());
                write_cache.insert(inc, formatter.as_file_content(&code));
                function_count += 1;
            }
        }

        // 5. 编译物编触发器
        let mut object_trigger_count = 0usize;
        for &tt in TableType::ALL {
            let table = EditorTable::open(project, map_name, tt)?;
            for summary in table.list_summaries()? {
                if let Ok(content) = fs::read_to_string(&summary.file_path) {
                    if let Ok(Some(code)) =
                        Self::compile_object_triggers(&content, tt.name_cn(), summary.key, &formatter)
                    {
                        let safe_name = Self::make_valid_filename(&summary.name, true);
                        let inc = format!("y3-trigger/object/{}/{}.lua", tt.name_cn(), safe_name);
                        include_files.push(inc.clone());
                        write_cache.insert(inc, formatter.as_file_content(&code));
                        object_trigger_count += 1;
                    }
                }
            }
        }

        // 6. 生成 y3-trigger/init.lua
        let mut init_lines = vec![
            "GameAPI.disable_all_eca_triggers()".to_string(),
            "Func = {}".to_string(),
            String::new(),
        ];
        for inc in &include_files {
            let mod_path = inc.trim_end_matches(".lua").replace(['/', '\\'], ".");
            init_lines.push(format!("include {}", encode_lua_string(&mod_path)));
        }
        write_cache.insert(
            "y3-trigger/init.lua".to_string(),
            formatter.as_file_content(&init_lines.join("\n")),
        );

        // 7. 写入磁盘并清理旧文件
        let out_base_dir = out_map.script_path.join("y3-trigger");
        if out_base_dir.is_dir() {
            for entry in WalkDir::new(&out_base_dir).into_iter().flatten() {
                if entry.file_type().is_file() {
                    if let Ok(rel) = entry.path().strip_prefix(&out_map.script_path) {
                        let rel_norm = rel.to_string_lossy().replace('\\', "/");
                        if !write_cache.contains_key(&rel_norm) {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }

        let mut written_files = Vec::new();
        for (rel_inc, content) in &write_cache {
            let target_path = out_map.script_path.join(rel_inc);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let unchanged = fs::read_to_string(&target_path)
                .map(|old| old == *content)
                .unwrap_or(false);
            if !unchanged {
                fs::write(&target_path, content)?;
            }
            written_files.push(target_path);
        }

        Ok(EcaCompileReport {
            map_name: out_map.name.clone(),
            trigger_count,
            function_count,
            object_trigger_count,
            written_files,
        })
    }

    fn scan_json_files(dir: &Path) -> Vec<(PathBuf, String, String)> {
        let mut results = Vec::new();
        if !dir.is_dir() {
            return results;
        }
        let mut used_names = HashSet::new();
        for entry in WalkDir::new(dir).into_iter().flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let rel = match path.strip_prefix(dir) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let stem = rel.trim_end_matches(".json");
            let mut valid = Self::make_valid_filename(stem, false);
            if used_names.contains(&valid) {
                let mut i = 1;
                loop {
                    let cand = format!("{}_{}", valid, i);
                    if !used_names.contains(&cand) {
                        valid = cand;
                        break;
                    }
                    i += 1;
                }
            }
            used_names.insert(valid.clone());
            if let Ok(content) = fs::read_to_string(path) {
                results.push((path.to_path_buf(), valid, content));
            }
        }
        results
    }

    fn make_valid_filename(name: &str, include_slash: bool) -> String {
        let mut out = String::new();
        for c in name.trim_end_matches(".json").chars() {
            if matches!(c, '.' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r')
                || (include_slash && (c == '/' || c == '\\'))
            {
                out.push('_');
            } else {
                out.push(c);
            }
        }
        out
    }

    fn compile_global_variables(content: &str, formatter: &EcaFormatter) -> anyhow::Result<String> {
        let val: Value = serde_json::from_str(content)?;
        let plain = convert_tuple_format(&val);
        let items = match plain.as_array().or_else(|| plain.get("items")?.as_array()) {
            Some(arr) => arr,
            None => return Ok(String::new()),
        };
        let mut lines = vec!["G = {}".to_string()];
        for item in items {
            let key = match item.get("key").and_then(|v| v.as_str()) {
                Some(k) => k,
                None => continue,
            };
            let val_node = item.get("value").unwrap_or(&Value::Null);
            let val_lua = Self::compile_node(val_node, formatter).unwrap_or_else(|| "nil".into());
            lines.push(format!("G[{}] = {}", encode_lua_string(key), val_lua));
        }
        Ok(lines.join("\n"))
    }

    fn compile_eca_json(
        content: &str,
        is_function: bool,
        formatter: &EcaFormatter,
    ) -> anyhow::Result<String> {
        let val: Value = serde_json::from_str(content)?;
        let plain = convert_tuple_format(&val);
        if is_function {
            Self::compile_function_node(&plain, formatter)
        } else {
            Self::compile_trigger_node(&plain, None, None, formatter)
        }
    }

    fn compile_object_triggers(
        content: &str,
        object_type_cn: &str,
        object_key: u64,
        formatter: &EcaFormatter,
    ) -> anyhow::Result<Option<String>> {
        let val: Value = serde_json::from_str(content)?;
        let plain = convert_tuple_format(&val);
        let triggers = match plain.get("triggers").and_then(|t| t.as_object()) {
            Some(map) if !map.is_empty() => map,
            _ => return Ok(None),
        };
        let mut chunks = Vec::new();
        for (_, trig_val) in triggers {
            if let Ok(code) = Self::compile_trigger_node(
                trig_val,
                Some(object_type_cn),
                Some(object_key),
                formatter,
            ) {
                chunks.push(code);
            }
        }
        if chunks.is_empty() {
            Ok(None)
        } else {
            Ok(Some(chunks.join("\n\n")))
        }
    }

    fn compile_function_node(node: &Value, formatter: &EcaFormatter) -> anyhow::Result<String> {
        let name = node
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unnamed_func");
        let root = node.get("root").unwrap_or(node);
        let body = Self::compile_node(root, formatter).unwrap_or_default();
        Ok(format!(
            "Func[{}] = function (...)\n{}\nend",
            encode_lua_string(name),
            body
        ))
    }

    fn compile_trigger_node(
        node: &Value,
        object_type: Option<&str>,
        object_key: Option<u64>,
        formatter: &EcaFormatter,
    ) -> anyhow::Result<String> {
        let enable = node.get("enable").and_then(|v| v.as_bool()).unwrap_or(true);
        let root = node.get("root").unwrap_or(node);
        let body = Self::compile_node(root, formatter).unwrap_or_default();

        let events = node
            .get("event")
            .or_else(|| node.get("events"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if events.is_empty() {
            return Ok(format!(
                "y3.eca.def {{\n    enable = {},\n}}\n: action(function (params)\n{}\nend)",
                enable, body
            ));
        }

        let mut ev_lines = Vec::new();
        for ev in &events {
            let ev_type = ev
                .get("type")
                .or_else(|| ev.get("event"))
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN_EVENT");
            let ev_name = formatter
                .event_names
                .get(ev_type)
                .cloned()
                .unwrap_or_else(|| ev_type.to_string());
            let mut ev_args = vec![encode_lua_string(&ev_name)];
            if let Some(args_arr) = ev.get("args").and_then(|v| v.as_array()) {
                for a in args_arr {
                    if let Some(compiled) = Self::compile_node(a, formatter) {
                        ev_args.push(compiled);
                    }
                }
            }
            ev_lines.push(format!(": event({})", ev_args.join(", ")));
        }

        let mut def_fields = vec![format!("    enable = {},", enable)];
        if let (Some(ot), Some(ok)) = (object_type, object_key) {
            def_fields.push(format!("    object_type = {},", encode_lua_string(ot)));
            def_fields.push(format!("    object_key = {},", ok));
        }

        Ok(format!(
            "y3.eca.def {{\n{}\n}}\n{}\n: action(function (params)\n{}\nend)",
            def_fields.join("\n"),
            ev_lines.join("\n"),
            body
        ))
    }

    fn compile_node(node: &Value, formatter: &EcaFormatter) -> Option<String> {
        match node {
            Value::Null => Some("nil".to_string()),
            Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
            Value::Number(_) | Value::String(_) => Some(encode_lua_value(node)),
            Value::Array(arr) => {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|item| Self::compile_node(item, formatter))
                    .filter(|s| !s.is_empty())
                    .collect();
                Some(parts.join("\n"))
            }
            Value::Object(map) => {
                // Trunk / Block 节点
                if let Some(Value::Array(actions)) =
                    map.get("actions").or_else(|| map.get("children"))
                {
                    let mut lines = Vec::new();
                    if let Some(Value::Array(conds)) = map.get("conditions") {
                        for c in conds {
                            if let Some(cond_str) = Self::compile_node(c, formatter) {
                                lines.push(format!("if not ({}) then return end", cond_str));
                            }
                        }
                    }
                    for act in actions {
                        if let Some(s) = Self::compile_node(act, formatter) {
                            if !s.is_empty() {
                                lines.push(s);
                            }
                        }
                    }
                    return Some(lines.join("\n"));
                }

                let node_type = map
                    .get("type")
                    .or_else(|| map.get("action"))
                    .or_else(|| map.get("func"))
                    .and_then(|v| {
                        v.as_str()
                            .map(|s| s.to_string())
                            .or_else(|| Some(v.to_string()))
                    })?;

                // 特殊内置节点处理
                if node_type == "FLOAT" {
                    if let Some(v) = map.get("value").and_then(|v| v.as_f64()) {
                        return Some(format!("{:.1}", v));
                    }
                }
                if node_type == "IF_THEN_ELSE" {
                    if let Some(Value::Array(args)) = map.get("args") {
                        let cond = args
                            .first()
                            .and_then(|a| Self::compile_node(a, formatter))
                            .unwrap_or_else(|| "false".to_string());
                        let thens = args
                            .get(1)
                            .and_then(|a| Self::compile_node(a, formatter))
                            .unwrap_or_default();
                        let elses = args
                            .get(2)
                            .and_then(|a| Self::compile_node(a, formatter))
                            .unwrap_or_default();
                        let mut res = format!("if {} then\n", cond);
                        if !thens.is_empty() {
                            res.push_str(&thens);
                            res.push('\n');
                        }
                        if !elses.is_empty() {
                            res.push_str("else\n");
                            res.push_str(&elses);
                            res.push('\n');
                        }
                        res.push_str("end");
                        return Some(res);
                    }
                }
                if node_type == "CALL_TRIGGER_FUNC" {
                    if let Some(Value::Array(args)) = map.get("args") {
                        if let Some(first) = args.first() {
                            let id = first
                                .get("type")
                                .or_else(|| first.get("name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("func");
                            let fname = formatter
                                .func_names
                                .get(id)
                                .cloned()
                                .unwrap_or_else(|| id.to_string());
                            let call_args: Vec<String> = first
                                .get("args")
                                .and_then(|a| a.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .map(|x| {
                                            Self::compile_node(x, formatter)
                                                .unwrap_or_else(|| "nil".into())
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            return Some(format!(
                                "Func[{}]({})",
                                encode_lua_string(&fname),
                                call_args.join(", ")
                            ));
                        }
                    }
                }

                // 常规 Value 节点
                if map.contains_key("value") && !map.contains_key("args") {
                    let raw_val = map.get("value").unwrap_or(&Value::Null);
                    if let Some(rule) = formatter.rules.get(&node_type) {
                        match rule {
                            RuleKind::Null => return Some("nil".to_string()),
                            RuleKind::Dict(d) => {
                                let k = raw_val
                                    .as_str()
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| raw_val.to_string());
                                if let Some(mapped) = d.get(&k) {
                                    return Some(mapped.clone());
                                }
                            }
                            RuleKind::Template { tpl, arg_names } => {
                                let v_str = encode_lua_value(raw_val);
                                return Some(formatter.apply_template(
                                    tpl,
                                    arg_names.as_deref(),
                                    &[Some(v_str)],
                                ));
                            }
                        }
                    }
                    return Some(encode_lua_value(raw_val));
                }

                // Call 节点
                let compiled_args: Vec<Option<String>> = map
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|a| Self::compile_node(a, formatter))
                            .collect()
                    })
                    .unwrap_or_default();

                if let Some(rule) = formatter.rules.get(&node_type) {
                    match rule {
                        RuleKind::Null => return Some("nil".to_string()),
                        RuleKind::Dict(d) => {
                            if let Some(Some(first)) = compiled_args.first() {
                                if let Some(mapped) = d.get(first.trim_matches('"')) {
                                    return Some(mapped.clone());
                                }
                            }
                        }
                        RuleKind::Template { tpl, arg_names } => {
                            return Some(formatter.apply_template(
                                tpl,
                                arg_names.as_deref(),
                                &compiled_args,
                            ));
                        }
                    }
                }

                let arg_strs: Vec<String> = compiled_args
                    .into_iter()
                    .map(|a| a.unwrap_or_else(|| "nil".into()))
                    .collect();
                Some(format!(
                    "y3.eca.call({}, {})",
                    encode_lua_string(&node_type),
                    arg_strs.join(", ")
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eca_compile_map() {
        let tmp = tempfile::tempdir().unwrap();
        let proj_dir = tmp.path().join("Proj");
        let map_dir = proj_dir.join("maps/EntryMap");
        fs::create_dir_all(map_dir.join("global_trigger/trigger")).unwrap();
        fs::create_dir_all(map_dir.join("script")).unwrap();
        fs::write(
            proj_dir.join("header.project"),
            r#"{"entry_map": {"id": 1}}"#,
        )
        .unwrap();
        fs::write(map_dir.join("header.map"), r#"{"id": 1}"#).unwrap();
        fs::write(
            map_dir.join("globaltriggervariable.json"),
            r#"[{"key": "score", "value": 100}]"#,
        )
        .unwrap();
        fs::write(
            map_dir.join("global_trigger/trigger/欢迎触发器.json"),
            r#"{
                "enable": true,
                "event": [{"type": "EVENT_GAME_INIT", "args": []}],
                "root": {
                    "actions": [
                        {
                            "type": "PRINT_MESSAGE_ACTION_TO_DIALOG",
                            "args": [
                                {"type": "100175", "value": "3"},
                                {"type": "STR", "value": "Hello ECA!"}
                            ]
                        }
                    ]
                }
            }"#,
        )
        .unwrap();

        let ctx = ProjectContext::load_from_path(&proj_dir).unwrap();
        let report = EcaCompiler::compile_map(&ctx, None, None).unwrap();
        assert_eq!(report.trigger_count, 1);

        let init_lua = fs::read_to_string(map_dir.join("script/y3-trigger/init.lua")).unwrap();
        assert!(init_lua.contains("GameAPI.disable_all_eca_triggers()"));
        assert!(init_lua.contains("y3-trigger.trigger.欢迎触发器"));

        let trig_lua =
            fs::read_to_string(map_dir.join("script/y3-trigger/trigger/欢迎触发器.lua")).unwrap();
        assert!(trig_lua.contains("log.info(\"Hello ECA!\")"));
    }
}
