use crate::editor_table::table::{EditorTable, TableType};
use crate::env::ProjectContext;
use calamine::{open_workbook_auto, Data, Reader};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// Excel 工作表封装（100% 兼容原版 `y3.excel.Sheet` 的坐标体系与解析算法）
#[derive(Debug, Clone)]
pub struct ExcelSheet {
    pub sheet_name: String,
    rows: Vec<Vec<String>>,
}

impl ExcelSheet {
    /// 从 `.xlsx` / `.xls` / `.ods` 文件加载指定工作表（若 `sheet_name` 为 None 则加载第一个工作表）
    pub fn load_file(file_path: &Path, sheet_name: Option<&str>) -> anyhow::Result<Self> {
        let mut workbook = open_workbook_auto(file_path)
            .map_err(|e| anyhow::anyhow!("无法打开 Excel 文件 {}: {}", file_path.display(), e))?;

        let target_name = match sheet_name {
            Some(name) => name.to_string(),
            None => workbook
                .sheet_names()
                .first()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Excel 文件没有任何工作表"))?,
        };

        let range = workbook
            .worksheet_range(&target_name)
            .map_err(|e| anyhow::anyhow!("读取工作表 '{}' 失败: {}", target_name, e))?;

        let mut rows = Vec::new();
        for row in range.rows() {
            let mut r = Vec::with_capacity(row.len());
            for cell in row {
                let s = match cell {
                    Data::Empty => String::new(),
                    Data::String(s) => s.clone(),
                    Data::Int(i) => i.to_string(),
                    Data::Float(f) => {
                        if f.fract() == 0.0 {
                            (*f as i64).to_string()
                        } else {
                            f.to_string()
                        }
                    }
                    Data::Bool(b) => if *b { "true" } else { "false" }.to_string(),
                    Data::DateTime(dt) => dt.to_string(),
                    Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
                    Data::Error(_) => String::new(),
                };
                r.push(s);
            }
            rows.push(r);
        }

        Ok(Self {
            sheet_name: target_name,
            rows,
        })
    }

    /// 从二维字符串数组直接构造（方便单元测试与内存表格）
    pub fn from_rows(sheet_name: &str, rows: Vec<Vec<String>>) -> Self {
        Self {
            sheet_name: sheet_name.to_string(),
            rows,
        }
    }

    /// 读取单元格（1-based: `col >= 1, row >= 1`）
    pub fn get_cell(&self, col: usize, row: usize) -> &str {
        if col == 0 || row == 0 {
            return "";
        }
        self.rows
            .get(row - 1)
            .and_then(|r| r.get(col - 1))
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// 将 Excel 坐标如 `"B1"`, `"AA12"` 解析为 1-based `(col, row)`
    pub fn parse_pos(pos: &str) -> anyhow::Result<(usize, usize)> {
        let mut col: usize = 0;
        let mut row_str = String::new();
        for c in pos.trim().chars() {
            if c.is_ascii_alphabetic() {
                let upper = c.to_ascii_uppercase() as u8;
                col = col * 26 + (upper - b'A' + 1) as usize;
            } else if c.is_ascii_digit() {
                row_str.push(c);
            }
        }
        let row: usize = row_str
            .parse()
            .map_err(|_| anyhow::anyhow!("非法的单元格坐标: {}", pos))?;
        if col == 0 || row == 0 {
            anyhow::bail!("非法的单元格坐标: {}", pos);
        }
        Ok((col, row))
    }

    /// 自动探测表格起始偏移 `(col, row)`（与原版 `Sheet.guessTableOffset` 完全一致：在 10x10 范围内找第一个连续右侧和下方都有值的格子）
    pub fn guess_table_offset(&self) -> (usize, usize) {
        for x in 1..=10 {
            for y in 1..=10 {
                if !self.get_cell(x, y).is_empty()
                    && !self.get_cell(x + 1, y).is_empty()
                    && !self.get_cell(x, y + 1).is_empty()
                {
                    return (x, y);
                }
            }
        }
        (1, 1)
    }

    /// 生成一维键值表：`Record<Key, Record<HeaderName, String>>`（与原版 `Sheet.makeTable` 完全一致）
    pub fn make_table(
        &self,
        offset: Option<(usize, usize)>,
        skip_rows: usize,
    ) -> BTreeMap<String, BTreeMap<String, String>> {
        let (ox, oy) = offset.unwrap_or_else(|| self.guess_table_offset());
        let mut titles: Vec<(usize, String)> = Vec::new();
        for x in ox..=100 {
            let t = self.get_cell(x, oy).trim();
            if t.is_empty() {
                break;
            }
            titles.push((x, t.to_string()));
        }

        let mut table = BTreeMap::new();
        for y in (oy + 1 + skip_rows)..=100000 {
            let key = self.get_cell(ox, y).trim();
            if key.is_empty() {
                break;
            }
            let mut row_map = BTreeMap::new();
            for (x, title) in &titles {
                row_map.insert(title.clone(), self.get_cell(*x, y).to_string());
            }
            table.insert(key.to_string(), row_map);
        }
        table
    }

    /// 生成一对多多维表：`Record<Key, Vec<Record<HeaderName, String>>>`（与原版 `Sheet.makeMultiTable` 完全一致）
    pub fn make_multi_table(
        &self,
        offset: Option<(usize, usize)>,
        skip_rows: usize,
    ) -> BTreeMap<String, Vec<BTreeMap<String, String>>> {
        let (ox, oy) = offset.unwrap_or_else(|| self.guess_table_offset());
        let mut titles: Vec<(usize, String)> = Vec::new();
        for x in ox..=100 {
            let t = self.get_cell(x, oy).trim();
            if t.is_empty() {
                break;
            }
            titles.push((x, t.to_string()));
        }

        let mut table: BTreeMap<String, Vec<BTreeMap<String, String>>> = BTreeMap::new();
        let mut current_key = String::new();
        for y in (oy + 1 + skip_rows)..=100000 {
            let raw_key = self.get_cell(ox, y).trim();
            if !raw_key.is_empty() {
                current_key = raw_key.to_string();
            }
            // 检查整行是否全空
            let all_empty = titles.iter().all(|(x, _)| self.get_cell(*x, y).trim().is_empty());
            if all_empty {
                break;
            }
            if current_key.is_empty() {
                continue;
            }
            let mut row_map = BTreeMap::new();
            for (x, title) in &titles {
                row_map.insert(title.clone(), self.get_cell(*x, y).to_string());
            }
            table.entry(current_key.clone()).or_default().push(row_map);
        }
        table
    }
}

/// 导表规则配置（支持通过 JSON / CLI 配置将 Excel 工作表批量导入到物编表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcelImportRule {
    /// 目标物编表名（如 `"单位"` / `"unit"` / `"技能"`）
    pub table_type: String,
    /// Excel 文件路径
    pub excel_path: String,
    /// 工作表名称（可选，默认第一个）
    pub sheet_name: Option<String>,
    /// 表格左上角锚点（如 `"A1"`, `"B1"`，可选，默认自动探测）
    pub offset: Option<String>,
    /// 表头下方跳过的注释行数（默认 0）
    #[serde(default)]
    pub skip_rows: usize,
    /// 作为模板复制基底的默认 key（可选）
    pub template_key: Option<u64>,
    /// 列名到物编字段名的映射（若为空则直接按列名匹配物编字段英文名或中文描述）
    #[serde(default)]
    pub field_map: BTreeMap<String, String>,
    /// 列名到自定义属性 `kv` 键名的映射
    #[serde(default)]
    pub kv_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExcelImportResult {
    pub table_type: String,
    pub total_rows: usize,
    pub updated_keys: Vec<u64>,
}

/// 执行 Excel 导表规则：读取表格并批量创建/更新物编对象
pub fn apply_excel_rule(
    project: &ProjectContext,
    map_name: Option<&str>,
    rule: &ExcelImportRule,
) -> anyhow::Result<ExcelImportResult> {
    let table_type = TableType::parse(&rule.table_type)
        .ok_or_else(|| anyhow::anyhow!("未知的物编表类型: {}", rule.table_type))?;
    let editor_table = EditorTable::open(project, map_name, table_type)?;

    let excel_path = Path::new(&rule.excel_path);
    let resolved_excel = if excel_path.is_absolute() {
        excel_path.to_path_buf()
    } else {
        project.project_path.join(excel_path)
    };

    let sheet = ExcelSheet::load_file(&resolved_excel, rule.sheet_name.as_deref())?;
    let offset = match &rule.offset {
        Some(pos) => Some(ExcelSheet::parse_pos(pos)?),
        None => None,
    };
    let rows = sheet.make_table(offset, rule.skip_rows);

    let existing_keys: std::collections::HashSet<u64> =
        editor_table.list_keys()?.into_iter().collect();
    let mut updated_keys = Vec::new();

    for (key_str, row) in &rows {
        let key: u64 = match key_str.parse() {
            Ok(k) => k,
            Err(_) => continue,
        };

        let mut fields = Map::new();
        let mut kv = Map::new();
        let mut obj_name = row
            .get("name")
            .or_else(|| row.get("名称"))
            .cloned()
            .unwrap_or_else(|| key.to_string());

        for (col_name, cell_str) in row {
            if cell_str.is_empty() {
                continue;
            }
            let parsed_val = parse_cell_value(cell_str);
            if rule.kv_columns.contains(col_name) || col_name.starts_with("kv.") {
                let kv_key = col_name.strip_prefix("kv.").unwrap_or(col_name);
                kv.insert(kv_key.to_string(), parsed_val);
                continue;
            }

            let target_field = rule
                .field_map
                .get(col_name)
                .cloned()
                .unwrap_or_else(|| col_name.clone());

            if target_field == "key" || target_field == "uid" || target_field == "编号" {
                continue;
            }
            if target_field == "name" || target_field == "名称" {
                obj_name = cell_str.clone();
                fields.insert("name".to_string(), Value::String(cell_str.clone()));
            } else {
                fields.insert(target_field, parsed_val);
            }
        }

        if existing_keys.contains(&key) {
            editor_table.update(key, Some(&fields), Some(&kv))?;
        } else {
            editor_table.create(
                Some(key),
                &obj_name,
                rule.template_key,
                Some(&fields),
                Some(&kv),
            )?;
        }
        updated_keys.push(key);
    }

    Ok(ExcelImportResult {
        table_type: table_type.name_cn().to_string(),
        total_rows: rows.len(),
        updated_keys,
    })
}

fn parse_cell_value(cell: &str) -> Value {
    let trimmed = cell.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return json!(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return json!(false);
    }
    if let Ok(i) = trimmed.parse::<i64>() {
        return json!(i);
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        return json!(f);
    }
    if (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('{') && trimmed.ends_with('}'))
    {
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            return v;
        }
    }
    Value::String(cell.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_excel_sheet_make_table() {
        let rows = vec![
            vec![
                "编号".to_string(),
                "名称".to_string(),
                "ori_bits".to_string(),
            ],
            vec![
                "134217729".to_string(),
                "步兵".to_string(),
                "5".to_string(),
            ],
            vec![
                "134217730".to_string(),
                "骑兵".to_string(),
                "8".to_string(),
            ],
        ];
        let sheet = ExcelSheet::from_rows("单位", rows);
        assert_eq!(sheet.guess_table_offset(), (1, 1));
        let table = sheet.make_table(None, 0);
        assert_eq!(table.len(), 2);
        assert_eq!(table["134217729"]["名称"], "步兵");
        assert_eq!(table["134217730"]["ori_bits"], "8");
    }
}
