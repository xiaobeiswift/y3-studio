use crate::env::ProjectContext;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// 生成 `y3/doc/API.md` 与 `y3/doc/API/{ClassName}.md`（对应原版 `src/makeLuaDoc.ts`）
pub struct LuaDocGenerator;

impl LuaDocGenerator {
    pub fn generate_from_doc_json(
        project: &ProjectContext,
        map_name: Option<&str>,
        doc_json_path: Option<&Path>,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let map = project.resolve_map(map_name)?;
        let y3_dir = map.script_path.join("y3");
        let doc_dir = y3_dir.join("doc");
        let api_dir = doc_dir.join("API");
        fs::create_dir_all(&api_dir)?;

        let json_file = doc_json_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| doc_dir.join("doc.json"));
        if !json_file.is_file() {
            anyhow::bail!("未找到 doc.json: {}", json_file.display());
        }

        let content = fs::read_to_string(&json_file)?;
        let classes: Value = serde_json::from_str(&content)?;
        let arr = classes
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("doc.json 顶层不是数组"))?;

        let mut index_lines = vec!["# API 目录\n".to_string()];
        let mut generated = Vec::new();

        for cls in arr {
            let name = match cls.get("name").and_then(|v| v.as_str()) {
                Some(n) => n,
                None => continue,
            };
            if name.starts_with("py.") || name.starts_with("Class") {
                continue;
            }
            let desc = cls
                .get("desc")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut md = vec![format!("# {}\n", name)];
            if !desc.is_empty() {
                md.push(format!("{}\n", desc));
            }

            if let Some(fields) = cls.get("fields").and_then(|v| v.as_array()) {
                for field in fields {
                    let fname = field.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let fdesc = field
                        .get("desc")
                        .or_else(|| field.get("rawdesc"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let ftype = field
                        .get("extends")
                        .and_then(|e| e.get("view"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("any");
                    md.push(format!("## `{}`\n\n- 类型: `{}`\n\n{}\n", fname, ftype, fdesc));
                }
            }

            let cls_path = api_dir.join(format!("{}.md", name));
            fs::write(&cls_path, md.join("\n"))?;
            generated.push(cls_path);
            index_lines.push(format!("- [{}](API/{}.md)", name, name));
        }

        let api_index_path = doc_dir.join("API.md");
        fs::write(&api_index_path, index_lines.join("\n"))?;
        generated.push(api_index_path);
        Ok(generated)
    }
}
