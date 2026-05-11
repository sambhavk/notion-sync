use serde_json::{json, Value};

const MAX_RICH_TEXT_LEN: usize = 1900;

pub fn parse(text: &str) -> Vec<Value> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks: Vec<Value> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // fenced code block
        if line.trim_start().starts_with("```") {
            let lang = line.trim_start().trim_start_matches('`').trim().to_string();
            let lang = if lang.is_empty() { "plain text".to_string() } else { lang };
            let mut code_lines: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            let code = code_lines.join("\n");
            blocks.push(code_block(&code, &lang));
            i += 1;
            continue;
        }

        // divider
        if matches!(line.trim(), "---" | "***" | "___") {
            blocks.push(json!({ "type": "divider", "divider": {} }));
            i += 1;
            continue;
        }

        // table
        if line.trim_start().starts_with('|') {
            let mut table_lines: Vec<&str> = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                table_lines.push(lines[i]);
                i += 1;
            }
            if let Some(b) = parse_table(&table_lines) {
                blocks.push(b);
            }
            continue;
        }

        // blockquote
        if let Some(rest) = line.trim_start().strip_prefix("> ") {
            blocks.push(quote_block(rest));
            i += 1;
            continue;
        }
        if line.trim() == ">" {
            blocks.push(quote_block(""));
            i += 1;
            continue;
        }

        // headings
        if let Some(rest) = line.strip_prefix("#### ").or_else(|| line.strip_prefix("##### ")).or_else(|| line.strip_prefix("###### ")) {
            blocks.push(heading(3, rest));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("### ") {
            blocks.push(heading(3, rest));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            blocks.push(heading(2, rest));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            blocks.push(heading(1, rest));
            i += 1;
            continue;
        }

        // todo list
        if let Some(rest) = line.trim_start().strip_prefix("- [ ] ").or_else(|| line.trim_start().strip_prefix("- [ ]")) {
            let (indent_blocks, advance) = collect_list_children(&lines, i + 1);
            let mut block = json!({
                "type": "to_do",
                "to_do": { "rich_text": inline(rest), "checked": false }
            });
            if !indent_blocks.is_empty() {
                block["to_do"]["children"] = json!(indent_blocks);
            }
            blocks.push(block);
            i += 1 + advance;
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("- [x] ").or_else(|| line.trim_start().strip_prefix("- [x]")) {
            let (indent_blocks, advance) = collect_list_children(&lines, i + 1);
            let mut block = json!({
                "type": "to_do",
                "to_do": { "rich_text": inline(rest), "checked": true }
            });
            if !indent_blocks.is_empty() {
                block["to_do"]["children"] = json!(indent_blocks);
            }
            blocks.push(block);
            i += 1 + advance;
            continue;
        }

        // unordered list
        if let Some(rest) = line.trim_start().strip_prefix("- ")
            .or_else(|| line.trim_start().strip_prefix("* "))
            .or_else(|| line.trim_start().strip_prefix("+ "))
        {
            let (indent_blocks, advance) = collect_list_children(&lines, i + 1);
            let mut block = json!({
                "type": "bulleted_list_item",
                "bulleted_list_item": { "rich_text": inline(rest) }
            });
            if !indent_blocks.is_empty() {
                block["bulleted_list_item"]["children"] = json!(indent_blocks);
            }
            blocks.push(block);
            i += 1 + advance;
            continue;
        }

        // ordered list
        if is_ordered_list_item(line) {
            let rest = line.trim_start().splitn(2, ". ").nth(1).unwrap_or("").trim_end();
            let (indent_blocks, advance) = collect_list_children(&lines, i + 1);
            let mut block = json!({
                "type": "numbered_list_item",
                "numbered_list_item": { "rich_text": inline(rest) }
            });
            if !indent_blocks.is_empty() {
                block["numbered_list_item"]["children"] = json!(indent_blocks);
            }
            blocks.push(block);
            i += 1 + advance;
            continue;
        }

        // image
        if let Some(rest) = line.trim_start().strip_prefix("![") {
            if let Some(url) = extract_image_url(rest) {
                if url.starts_with("http://") || url.starts_with("https://") {
                    blocks.push(json!({
                        "type": "image",
                        "image": { "type": "external", "external": { "url": url } }
                    }));
                    i += 1;
                    continue;
                }
            }
        }

        // blank line — skip
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // paragraph — collect continuation lines
        let mut para_text = line.to_string();
        while i + 1 < lines.len() {
            let next = lines[i + 1];
            if next.trim().is_empty()
                || next.trim_start().starts_with('#')
                || next.trim_start().starts_with("```")
                || next.trim_start().starts_with('|')
                || next.trim_start().starts_with("- ")
                || next.trim_start().starts_with("* ")
                || next.trim_start().starts_with("+ ")
                || next.trim_start().starts_with("> ")
                || is_ordered_list_item(next)
                || matches!(next.trim(), "---" | "***" | "___")
            {
                break;
            }
            para_text.push(' ');
            para_text.push_str(next.trim());
            i += 1;
        }

        // split long paragraphs
        for part in split_long_text(&para_text) {
            blocks.push(json!({
                "type": "paragraph",
                "paragraph": { "rich_text": inline(&part) }
            }));
        }
        i += 1;
    }

    blocks
}

fn collect_list_children<'a>(lines: &[&'a str], start: usize) -> (Vec<Value>, usize) {
    let mut children: Vec<Value> = Vec::new();
    let mut j = start;
    while j < lines.len() {
        let l = lines[j];
        let indent = leading_spaces(l);
        if indent < 2 {
            break;
        }
        let trimmed = l.trim_start();
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("+ ")) {
            children.push(json!({
                "type": "bulleted_list_item",
                "bulleted_list_item": { "rich_text": inline(rest) }
            }));
            j += 1;
        } else if is_ordered_list_item(trimmed) {
            let rest = trimmed.splitn(2, ". ").nth(1).unwrap_or("").trim_end();
            children.push(json!({
                "type": "numbered_list_item",
                "numbered_list_item": { "rich_text": inline(rest) }
            }));
            j += 1;
        } else {
            break;
        }
    }
    (children, j - start)
}

fn leading_spaces(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

fn is_ordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    let mut chars = trimmed.chars();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            continue;
        }
        return c == '.' && trimmed.contains(". ");
    }
    false
}

fn extract_image_url(rest: &str) -> Option<String> {
    // rest is after "!["
    let close_bracket = rest.find(']')?;
    let after = &rest[close_bracket + 1..];
    let after = after.strip_prefix('(')?;
    let close_paren = after.find(')')?;
    Some(after[..close_paren].split_whitespace().next()?.to_string())
}

fn heading(level: u8, text: &str) -> Value {
    let t = match level {
        1 => "heading_1",
        2 => "heading_2",
        _ => "heading_3",
    };
    json!({
        "type": t,
        t: { "rich_text": inline(text) }
    })
}

fn quote_block(text: &str) -> Value {
    json!({
        "type": "quote",
        "quote": { "rich_text": inline(text) }
    })
}

fn code_block(code: &str, lang: &str) -> Value {
    let content = if code.len() > MAX_RICH_TEXT_LEN {
        &code[..MAX_RICH_TEXT_LEN]
    } else {
        code
    };
    json!({
        "type": "code",
        "code": {
            "rich_text": [{ "type": "text", "text": { "content": content } }],
            "language": lang
        }
    })
}

fn parse_table(lines: &[&str]) -> Option<Value> {
    let data_rows: Vec<Vec<&str>> = lines
        .iter()
        .filter(|l| !is_separator_row(l))
        .map(|l| parse_table_row(l))
        .collect();

    if data_rows.is_empty() {
        return None;
    }

    let width = data_rows[0].len();
    let mut rows: Vec<Value> = Vec::new();
    for row in &data_rows {
        let cells: Vec<Value> = row
            .iter()
            .map(|cell| json!(inline(cell.trim())))
            .collect();
        // pad or trim to width
        let mut cells = cells;
        cells.resize(width, json!([]));
        rows.push(json!({
            "type": "table_row",
            "table_row": { "cells": cells }
        }));
    }

    Some(json!({
        "type": "table",
        "table": {
            "table_width": width,
            "has_column_header": true,
            "has_row_header": false,
            "children": rows
        }
    }))
}

fn is_separator_row(line: &str) -> bool {
    line.trim().chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
        && line.contains('-')
}

fn parse_table_row(line: &str) -> Vec<&str> {
    let trimmed = line.trim().trim_start_matches('|').trim_end_matches('|');
    trimmed.split('|').collect()
}

fn split_long_text(text: &str) -> Vec<String> {
    if text.len() <= MAX_RICH_TEXT_LEN {
        return vec![text.to_string()];
    }
    let mut parts = Vec::new();
    let mut remaining = text;
    while remaining.len() > MAX_RICH_TEXT_LEN {
        let slice = &remaining[..MAX_RICH_TEXT_LEN];
        let cut = slice.rfind(". ").map(|p| p + 2).unwrap_or(MAX_RICH_TEXT_LEN);
        parts.push(remaining[..cut].to_string());
        remaining = &remaining[cut..];
    }
    if !remaining.is_empty() {
        parts.push(remaining.to_string());
    }
    parts
}

pub fn inline(text: &str) -> Value {
    let spans = parse_inline(text);
    json!(spans)
}

#[derive(Debug, Default, Clone)]
struct Span {
    content: String,
    bold: bool,
    italic: bool,
    strikethrough: bool,
    code: bool,
    href: Option<String>,
}

impl Span {
    fn to_json(&self) -> Value {
        let mut obj = json!({
            "type": "text",
            "text": { "content": &self.content },
            "annotations": {
                "bold": self.bold,
                "italic": self.italic,
                "strikethrough": self.strikethrough,
                "code": self.code,
                "underline": false,
                "color": "default"
            }
        });
        if let Some(ref url) = self.href {
            obj["text"]["link"] = json!({ "url": url });
        }
        obj
    }
}

fn parse_inline(text: &str) -> Vec<Value> {
    let mut spans: Vec<Value> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut current = Span::default();

    macro_rules! flush {
        () => {
            if !current.content.is_empty() {
                // split if too long
                let s = current.clone();
                for part in split_long_text(&s.content) {
                    let mut sp = s.clone();
                    sp.content = part;
                    spans.push(sp.to_json());
                }
                current.content.clear();
            }
        };
    }

    while i < chars.len() {
        // inline code
        if chars[i] == '`' {
            flush!();
            i += 1;
            let mut code_content = String::new();
            while i < chars.len() && chars[i] != '`' {
                code_content.push(chars[i]);
                i += 1;
            }
            if i < chars.len() { i += 1; }
            spans.push(Span { content: code_content, code: true, ..Default::default() }.to_json());
            continue;
        }

        // link [text](url)
        if chars[i] == '[' {
            let start = i + 1;
            if let Some(close) = chars[start..].iter().position(|&c| c == ']') {
                let after = start + close + 1;
                if after < chars.len() && chars[after] == '(' {
                    if let Some(url_end) = chars[after + 1..].iter().position(|&c| c == ')') {
                        let link_text: String = chars[start..start + close].iter().collect();
                        let url: String = chars[after + 1..after + 1 + url_end].iter().collect();
                        flush!();
                        // Notion only accepts http/https URLs — render local/relative links as plain text
                        let href = if url.starts_with("http://") || url.starts_with("https://") {
                            Some(url)
                        } else {
                            None
                        };
                        spans.push(Span { content: link_text, href, ..Default::default() }.to_json());
                        i = after + 1 + url_end + 1;
                        continue;
                    }
                }
            }
        }

        // bold+italic ***
        if chars.len() > i + 2 && chars[i] == '*' && chars[i+1] == '*' && chars[i+2] == '*' {
            flush!();
            i += 3;
            let mut inner = String::new();
            while i + 2 < chars.len() && !(chars[i] == '*' && chars[i+1] == '*' && chars[i+2] == '*') {
                inner.push(chars[i]);
                i += 1;
            }
            i += 3;
            spans.push(Span { content: inner, bold: true, italic: true, ..Default::default() }.to_json());
            continue;
        }

        // bold **
        if chars.len() > i + 1 && chars[i] == '*' && chars[i+1] == '*' {
            flush!();
            i += 2;
            let mut inner = String::new();
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i+1] == '*') {
                inner.push(chars[i]);
                i += 1;
            }
            i += 2;
            spans.push(Span { content: inner, bold: true, ..Default::default() }.to_json());
            continue;
        }

        // bold __
        if chars.len() > i + 1 && chars[i] == '_' && chars[i+1] == '_' {
            flush!();
            i += 2;
            let mut inner = String::new();
            while i + 1 < chars.len() && !(chars[i] == '_' && chars[i+1] == '_') {
                inner.push(chars[i]);
                i += 1;
            }
            i += 2;
            spans.push(Span { content: inner, bold: true, ..Default::default() }.to_json());
            continue;
        }

        // strikethrough ~~
        if chars.len() > i + 1 && chars[i] == '~' && chars[i+1] == '~' {
            flush!();
            i += 2;
            let mut inner = String::new();
            while i + 1 < chars.len() && !(chars[i] == '~' && chars[i+1] == '~') {
                inner.push(chars[i]);
                i += 1;
            }
            i += 2;
            spans.push(Span { content: inner, strikethrough: true, ..Default::default() }.to_json());
            continue;
        }

        // italic *
        if chars[i] == '*' {
            flush!();
            i += 1;
            let mut inner = String::new();
            while i < chars.len() && chars[i] != '*' {
                inner.push(chars[i]);
                i += 1;
            }
            if i < chars.len() { i += 1; }
            spans.push(Span { content: inner, italic: true, ..Default::default() }.to_json());
            continue;
        }

        // italic _
        if chars[i] == '_' {
            flush!();
            i += 1;
            let mut inner = String::new();
            while i < chars.len() && chars[i] != '_' {
                inner.push(chars[i]);
                i += 1;
            }
            if i < chars.len() { i += 1; }
            spans.push(Span { content: inner, italic: true, ..Default::default() }.to_json());
            continue;
        }

        current.content.push(chars[i]);
        i += 1;
    }
    flush!();

    if spans.is_empty() {
        spans.push(Span { content: String::new(), ..Default::default() }.to_json());
    }
    spans
}
