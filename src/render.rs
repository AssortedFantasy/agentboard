use crate::model::{Output, Receipt};
use anyhow::{Result, bail};
use serde_json::{Value, json};

pub struct Rendered {
    pub text: String,
    pub receipts: Vec<Receipt>,
    pub notification_ids: Vec<i64>,
}

fn scalar(value: &Value) -> String {
    match value { Value::String(s) => s.clone(), Value::Null => "-".into(), _ => value.to_string() }
}

fn text_item(item: &Value, compact: bool) -> String {
    let Some(map) = item.as_object() else { return format!("{}\n", scalar(item)); };
    let mut out = String::new();
    if let Some(id) = item["id"].as_i64().filter(|_| item.get("kind").is_some()) {
        out.push_str(&format!("#{id} {} {}", scalar(&item["kind"]), item["title"].as_str().unwrap_or("")));
        if let Some(rev) = item["revision"].as_i64() { out.push_str(&format!(" [r{rev}]")); }
        out.push('\n');
    }
    for (key, value) in map {
        if key == "changes" && item.get("patch").is_some() { continue; }
        if matches!(key.as_str(), "id"|"kind"|"title"|"revision") && item.get("kind").is_some() { continue; }
        if key == "body" || key == "diff" || key == "patch" {
            if !compact { out.push_str(&format!("{key}:\n{}\n", scalar(value))); }
        } else if key == "items" || key == "comments" || key == "timeline" {
            if let Some(items) = value.as_array() { for child in items { out.push_str(&text_item(child, compact)); } }
            else { out.push_str(&format!("{key}: {}\n", scalar(value))); }
        } else if !value.is_null() && !(compact && matches!(key.as_str(),"created_at"|"updated_at"|"metadata"|"summary"|"author"|"forum_id"|"parent_id"|"path")) {
            out.push_str(&format!("{key}: {}\n", scalar(value)));
        }
    }
    out
}

fn serialize(kind: &str, items: &[Value], more: bool, notices: &[String], format: &str, compact: bool) -> Result<String> {
    Ok(match format {
        "json" => format!("{}\n", serde_json::to_string(&json!({"kind":kind,"items":items,"more":more,"notices":notices}))?),
        "jsonl" => {
            let mut text = String::new();
            for item in items { text.push_str(&serde_json::to_string(&json!({"type":"item","item":item}))?); text.push('\n'); }
            text.push_str(&serde_json::to_string(&json!({"type":"meta","kind":kind,"more":more,"notices":notices}))?); text.push('\n'); text
        },
        "text" => {
            let mut text = String::new();
            for (i,item) in items.iter().enumerate() { if i > 0 && !compact { text.push('\n'); } text.push_str(&text_item(item,compact)); }
            if items.is_empty() { text.push_str("No results.\n"); }
            for notice in notices { text.push_str(notice); text.push('\n'); }
            if more {
                if matches!(kind,"feed"|"inbox"|"notifications"|"updates") { text.push_str("More pending results remain. Run this command again; increase --max-bytes if a record was truncated.\n"); }
                else { text.push_str("More results omitted. Increase --limit/--max-bytes or continue with --offset.\n"); }
            }
            text
        },
        other => bail!("unknown output format {other}; use text, json or jsonl"),
    })
}

fn shorten(value: &mut Value, ceiling: usize) {
    match value {
        Value::String(s) if s.len() > ceiling => {
            let mut end = ceiling;
            while !s.is_char_boundary(end) { end -= 1; }
            s.truncate(end); s.push_str("… [truncated]");
        },
        Value::Object(map) => { for child in map.values_mut() { shorten(child, ceiling); } },
        Value::Array(items) => { for child in items { shorten(child, ceiling); } },
        _ => (),
    }
}

fn find_object(value: &Value, id: i64, revision: i64) -> bool {
    match value {
        Value::Object(map) => (value["id"].as_i64()==Some(id) && value["revision"].as_i64()==Some(revision)) || map.values().any(|child| find_object(child,id,revision)),
        Value::Array(items) => items.iter().any(|child| find_object(child,id,revision)),
        _ => false,
    }
}

fn contains_notification(value: &Value,id:i64) -> bool {
    value["notification_id"].as_i64()==Some(id)
}

/// Budget complete output, retaining receipts only for complete emitted records.
pub fn render(output: &Output, format: &str, compact: bool, max_bytes: usize) -> Result<Rendered> {
    if max_bytes < 1024 { bail!("--max-bytes must be at least 1024"); }
    let mut items = Vec::new();
    let mut complete = Vec::new();
    let mut notices = output.notices.clone();
    let mut more = output.more;
    for item in &output.items {
        items.push(item.clone());
        // Reserve room for the explicit omission message and final envelope.
        if serialize(&output.kind,&items,true,&notices,format,compact)?.len() <= max_bytes.saturating_sub(256) {
            complete.push(item.clone());
            continue;
        }
        items.pop();
        more = true;
        if items.is_empty() {
            let mut shortened = item.clone();
            let mut ceiling = max_bytes / 4;
            loop {
                shorten(&mut shortened, ceiling);
                if let Some(map)=shortened.as_object_mut() { map.insert("output_truncated".into(),Value::Bool(true)); }
                if serialize(&output.kind,std::slice::from_ref(&shortened),true,&notices,format,compact)?.len() <= max_bytes.saturating_sub(256) {
                    items.push(shortened); break;
                }
                if ceiling <= 16 { break; }
                ceiling /= 2;
            }
        }
        notices.push("Output budget reached; omitted or truncated records remain unread. Use --max-bytes for more content.".into());
        break;
    }
    let text = serialize(&output.kind,&items,more,&notices,format,compact)?;
    if text.len() > max_bytes { bail!("output metadata exceeds --max-bytes; increase the budget"); }
    let receipts = output.receipts.iter().filter(|r| complete.iter().any(|v| find_object(v,r.object_id,r.revision))).map(|r| Receipt { full: r.full && !compact, ..r.clone() }).collect();
    let notification_ids = output.notification_ids.iter().copied().filter(|id| complete.iter().any(|v| contains_notification(v,*id))).collect();
    Ok(Rendered { text, receipts, notification_ids })
}
