use crate::md_blocks::inline;
use serde_json::{json, Value};

pub fn parse(text: &str) -> Vec<Value> {
    let yaml: serde_yaml::Value = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            // fallback: render as code block
            return vec![json!({
                "type": "code",
                "code": {
                    "rich_text": [{"type":"text","text":{"content": &text[..text.len().min(1900)]}}],
                    "language": "yaml"
                }
            })];
        }
    };

    let mut blocks: Vec<Value> = Vec::new();

    // metadata header
    for key in &["file_name", "repo_type", "read_intent", "write_mode"] {
        if let Some(val) = yaml.get(key) {
            let val_str = yaml_val_to_string(val);
            blocks.push(kv_paragraph(key, &val_str));
        }
    }

    // consumers
    if let Some(consumers) = yaml.get("consumers").and_then(|v| v.as_sequence()) {
        blocks.push(json!({
            "type": "heading_2",
            "heading_2": { "rich_text": inline("Consumers") }
        }));
        for c in consumers {
            let text = yaml_val_to_string(c);
            blocks.push(json!({
                "type": "bulleted_list_item",
                "bulleted_list_item": { "rich_text": inline(&text) }
            }));
        }
    }

    // sections
    if let Some(sections) = yaml.get("sections").and_then(|v| v.as_sequence()) {
        for section in sections {
            blocks.extend(render_section(section, 2));
        }
    }

    blocks
}

fn render_section(section: &serde_yaml::Value, heading_level: u8) -> Vec<Value> {
    let mut blocks: Vec<Value> = Vec::new();

    let name = section.get("name").map(yaml_val_to_string).unwrap_or_default();
    let required = section.get("required").and_then(|v| v.as_bool());

    let heading_text = match required {
        Some(false) => format!("{name} (optional)"),
        _ => name.clone(),
    };

    blocks.push(heading_block(heading_level, &heading_text));

    if let Some(format_val) = section.get("format") {
        let s = yaml_val_to_string(format_val);
        blocks.push(kv_paragraph("format", &s));
    }

    if let Some(desc) = section.get("description") {
        let s = yaml_val_to_string(desc);
        blocks.push(json!({
            "type": "paragraph",
            "paragraph": { "rich_text": inline(&s) }
        }));
    }

    if let Some(columns) = section.get("columns").and_then(|v| v.as_sequence()) {
        let cols: Vec<String> = columns.iter().map(yaml_val_to_string).collect();
        let text = format!("Columns: {}", cols.join(" | "));
        blocks.push(json!({
            "type": "paragraph",
            "paragraph": { "rich_text": inline(&text) }
        }));
    }

    if let Some(omit_if) = section.get("omit_if") {
        let s = yaml_val_to_string(omit_if);
        blocks.push(json!({
            "type": "quote",
            "quote": { "rich_text": inline(&format!("Omit if: {s}")) }
        }));
    }

    if let Some(sub_sections) = section.get("sub_sections").and_then(|v| v.as_sequence()) {
        for sub in sub_sections {
            blocks.extend(render_section(sub, 3));
        }
    }

    blocks
}

fn heading_block(level: u8, text: &str) -> Value {
    let t = match level {
        1 => "heading_1",
        2 => "heading_2",
        _ => "heading_3",
    };
    json!({ "type": t, t: { "rich_text": inline(text) } })
}

fn kv_paragraph(key: &str, val: &str) -> Value {
    let rich_text = json!([
        { "type": "text", "text": { "content": format!("{key}: ") }, "annotations": { "bold": true, "italic": false, "strikethrough": false, "underline": false, "code": false, "color": "default" } },
        { "type": "text", "text": { "content": val }, "annotations": { "bold": false, "italic": false, "strikethrough": false, "underline": false, "code": false, "color": "default" } }
    ]);
    json!({ "type": "paragraph", "paragraph": { "rich_text": rich_text } })
}

fn yaml_val_to_string(val: &serde_yaml::Value) -> String {
    match val {
        serde_yaml::Value::String(s) => s.trim().to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => String::new(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .map(yaml_val_to_string)
            .collect::<Vec<_>>()
            .join(", "),
        serde_yaml::Value::Mapping(_) => serde_yaml::to_string(val).unwrap_or_default(),
        _ => String::new(),
    }
}
