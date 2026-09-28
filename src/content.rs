//! Durable content, immutable revisions, references and per-agent update views.
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::{
    db, events,
    model::{self, Output, Receipt, Request},
};

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}
fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn now() -> &'static str {
    "strftime('%Y-%m-%dT%H:%M:%fZ','now')"
}

/// SQL expression for inherited archive status. `alias` must be a trusted SQL alias.
pub fn archive_predicate(alias: &str) -> String {
    format!(
        "EXISTS(WITH RECURSIVE lineage(id,next,archived) AS (SELECT id,CASE kind WHEN 'post' THEN forum_id ELSE parent_id END,archived FROM objects WHERE id={alias}.id UNION ALL SELECT p.id,CASE p.kind WHEN 'post' THEN p.forum_id ELSE p.parent_id END,p.archived FROM objects p JOIN lineage l ON p.id=l.next) SELECT 1 FROM lineage WHERE archived=1)"
    )
}

pub fn effectively_archived(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.query_row(
        &format!(
            "SELECT {} FROM objects o WHERE o.id=?1",
            archive_predicate("o")
        ),
        [id],
        |r| r.get(0),
    )?)
}

pub fn resolve_forum(conn: &Connection, path: &str) -> Result<i64> {
    conn.query_row(
        "SELECT id FROM objects WHERE kind='forum' AND path=?1",
        [path],
        |r| r.get(0),
    )
    .optional()?
    .with_context(|| format!("forum {path:?} does not exist; create it with forum create"))
}

fn target(conn: &Connection, args: &Value) -> Result<i64> {
    if let Some(id) = args.get("id").and_then(Value::as_i64) {
        return Ok(id);
    }
    resolve_forum(conn, &model::string(args, "path")?)
}

fn require_kind(conn: &Connection, id: i64, kind: &str) -> Result<Value> {
    let object = db::get_object(conn, id)?;
    ensure!(object["kind"] == kind, "#{id} is not a {kind}");
    Ok(object)
}

fn tags(args: &Value) -> Result<Option<BTreeSet<String>>> {
    let Some(value) = args.get("tags") else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .context("tags must be an array of strings")?;
    let mut result = BTreeSet::new();
    for value in values {
        let value = value.as_str().context("tags must contain strings")?;
        ensure!(
            !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_whitespace),
            "tags must be nonempty, at most 128 bytes and contain no whitespace"
        );
        result.insert(value.to_owned());
    }
    Ok(Some(result))
}

fn metadata(args: &Value) -> Result<String> {
    let value = args.get("metadata").cloned().unwrap_or_else(|| json!({}));
    ensure!(value.is_object(), "metadata must be a JSON object");
    Ok(serde_json::to_string(&value)?)
}

fn replace_tags(conn: &Connection, id: i64, values: &BTreeSet<String>) -> Result<()> {
    conn.execute("DELETE FROM tags WHERE object_id=?1", [id])?;
    for tag in values {
        conn.execute(
            "INSERT INTO tags(object_id,tag) VALUES(?1,?2)",
            params![id, tag],
        )?;
    }
    Ok(())
}

/// Extract compact references with token boundaries, preserving authored text.
pub fn references(input: &str) -> BTreeSet<i64> {
    let mut result = BTreeSet::new();
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != '#' || (i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '#')) {
            continue;
        }
        let digits: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let end = i + 1 + digits.len();
        if end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            continue;
        }
        if let Ok(id) = digits.parse::<i64>()
            && id > 0
        {
            result.insert(id);
        }
    }
    result
}

fn sync_links(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let (title, body): (String, String) =
        conn.query_row("SELECT title,body FROM objects WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let old: BTreeSet<i64> = conn
        .prepare("SELECT target_id FROM links WHERE source_id=?1")?
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    conn.execute("DELETE FROM links WHERE source_id=?1", [id])?;
    let mut added = Vec::new();
    for target in references(&format!("{title}\n{body}")) {
        if target == id {
            continue;
        }
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE id=?1)",
            [target],
            |r| r.get(0),
        )?;
        if exists {
            conn.execute(
                "INSERT INTO links(source_id,target_id) VALUES(?1,?2)",
                params![id, target],
            )?;
            if !old.contains(&target) {
                added.push(target);
            }
        }
    }
    Ok(added)
}

pub fn emit_references(conn: &Connection, actor: &str, id: i64, added: &[i64]) -> Result<()> {
    let source = db::get_object(conn, id)?;
    for target in added {
        events::emit(
            conn,
            actor,
            "reference.added",
            *target,
            &json!({"source_id":id,"source_title":source["title"]}),
        )?;
    }
    Ok(())
}

/// Insert only; caller owns the transaction, final revision and event.
pub fn create_post(conn: &Connection, actor: &str, args: &Value) -> Result<i64> {
    let title = model::string(args, "title")?;
    ensure!(!title.trim().is_empty(), "post title must not be empty");
    let forum = resolve_forum(conn, text(args, "forum").unwrap_or("/"))?;
    let archived = effectively_archived(conn, forum)?;
    ensure!(
        !archived,
        "forum is archived; restore it before creating posts"
    );
    let meta = metadata(args)?;
    let values = tags(args)?.unwrap_or_default();
    conn.execute("INSERT INTO objects(kind,forum_id,title,body,summary,metadata,author) VALUES('post',?1,?2,?3,?4,?5,?6)",params![forum,title,text(args,"body").unwrap_or(""),text(args,"summary").unwrap_or(""),meta,actor])?;
    let id = conn.last_insert_rowid();
    replace_tags(conn, id, &values)?;
    sync_links(conn, id)?;
    Ok(id)
}

fn present(mut object: Value, full: bool) -> (Value, Receipt) {
    let receipt = Receipt {
        object_id: object["id"].as_i64().unwrap_or(0),
        revision: object["revision"].as_i64().unwrap_or(0),
        full,
    };
    if !full {
        object.as_object_mut().unwrap().remove("body");
        object["body_omitted"] = json!(true);
    }
    (object, receipt)
}

fn object_output(conn: &Connection, id: i64, full: bool) -> Result<Output> {
    let (object, receipt) = present(db::get_object(conn, id)?, full);
    let mut out = Output::one("objects", object);
    out.receipts.push(receipt);
    Ok(out)
}

fn create(conn: &mut Connection, actor: &str, req: &Request, kind: &str) -> Result<Output> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let args = &req.args;
    let id = match kind {
        "post" => create_post(&tx, actor, args)?,
        "forum" => {
            let path = model::string(args, "path")?;
            ensure!(
                path.starts_with('/')
                    && path != "/"
                    && !path.ends_with('/')
                    && !path.contains("//")
                    && !path
                        .split('/')
                        .any(|p| p == "." || p == ".." || p.chars().any(char::is_whitespace)),
                "forum path must be absolute, with nonempty segments and no trailing slash, whitespace, . or .."
            );
            let (parent_path, name) = path.rsplit_once('/').unwrap();
            let parent = resolve_forum(
                &tx,
                if parent_path.is_empty() {
                    "/"
                } else {
                    parent_path
                },
            )?;
            ensure!(
                !effectively_archived(&tx, parent)?,
                "parent forum is archived; restore it before creating subforums"
            );
            let meta = metadata(args)?;
            tx.execute("INSERT INTO objects(kind,parent_id,path,title,body,summary,metadata,author) VALUES('forum',?1,?2,?3,?4,?5,?6,?7)",params![parent,path,text(args,"title").unwrap_or(name),text(args,"body").unwrap_or(""),text(args,"summary").unwrap_or(""),meta,actor])?;
            let id = tx.last_insert_rowid();
            replace_tags(&tx, id, &tags(args)?.unwrap_or_default())?;
            sync_links(&tx, id)?;
            id
        }
        "comment" => {
            let post = model::id(args, "post")?;
            let parent = require_kind(&tx, post, "post")?;
            ensure!(
                !effectively_archived(&tx, post)?,
                "post #{post} is archived; restore it before commenting"
            );
            let reply = args.get("reply_to").and_then(Value::as_i64);
            if let Some(reply) = reply {
                let r = require_kind(&tx, reply, "comment")?;
                ensure!(
                    r["parent_id"] == post,
                    "reply #{reply} belongs to another post"
                );
            }
            let body = model::string(args, "body")?;
            ensure!(!body.trim().is_empty(), "comment body must not be empty");
            let meta = metadata(args)?;
            tx.execute("INSERT INTO objects(kind,forum_id,parent_id,reply_to,title,body,summary,metadata,author) VALUES('comment',?1,?2,?3,'',?4,?5,?6,?7)",params![parent["forum_id"].as_i64(),post,reply,body,text(args,"summary").unwrap_or(""),meta,actor])?;
            let id = tx.last_insert_rowid();
            replace_tags(&tx, id, &tags(args)?.unwrap_or_default())?;
            sync_links(&tx, id)?;
            id
        }
        _ => unreachable!(),
    };
    db::save_revision(&tx, id, actor)?;
    events::emit(&tx, actor, &format!("{kind}.created"), id, &json!({}))?;
    let refs: Vec<i64> = tx
        .prepare("SELECT target_id FROM links WHERE source_id=?1")?
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    emit_references(&tx, actor, id, &refs)?;
    let out = object_output(&tx, id, true)?;
    tx.commit()?;
    Ok(out)
}

fn edit(
    conn: &mut Connection,
    actor: &str,
    req: &Request,
    kind: Option<&str>,
    op: &str,
) -> Result<Output> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let args = &req.args;
    let id = target(&tx, args)?;
    let before = db::get_object(&tx, id)?;
    if let Some(kind) = kind {
        ensure!(before["kind"] == kind, "#{id} is not a {kind}");
    }
    if let Some(expected) = args.get("expected_revision") {
        ensure!(
            expected.as_i64() == before["revision"].as_i64(),
            "revision conflict for #{id}: expected {expected}, current {}; read current content and retry",
            before["revision"]
        );
    }
    let mut fields = Vec::new();
    match op {
        "archive" | "unarchive" => {
            ensure!(before["path"] != "/", "the root forum cannot be archived");
            tx.execute(
                "UPDATE objects SET archived=?1 WHERE id=?2",
                params![op == "archive", id],
            )?;
            fields.push("archived");
        }
        "tag.add" | "tag.remove" => {
            let values = tags(args)?.context("supply tags to add or remove")?;
            for tag in values {
                if op == "tag.add" {
                    tx.execute(
                        "INSERT OR IGNORE INTO tags(object_id,tag) VALUES(?1,?2)",
                        params![id, tag],
                    )?;
                } else {
                    tx.execute(
                        "DELETE FROM tags WHERE object_id=?1 AND tag=?2",
                        params![id, tag],
                    )?;
                }
            }
            fields.push("tags");
        }
        _ => {
            for key in ["title", "body", "summary"] {
                if let Some(value) = args.get(key) {
                    let value = value
                        .as_str()
                        .with_context(|| format!("{key} must be text"))?;
                    if key == "title" && before["kind"] != "comment" {
                        ensure!(!value.trim().is_empty(), "title must not be empty");
                    }
                    tx.execute(
                        &format!("UPDATE objects SET {key}=?1 WHERE id=?2"),
                        params![value, id],
                    )?;
                    fields.push(key);
                }
            }
            if args.get("metadata").is_some() {
                tx.execute(
                    "UPDATE objects SET metadata=?1 WHERE id=?2",
                    params![metadata(args)?, id],
                )?;
                fields.push("metadata");
            }
            if let Some(values) = tags(args)? {
                replace_tags(&tx, id, &values)?;
                fields.push("tags");
            }
            ensure!(
                !fields.is_empty(),
                "edit requires title, body, summary, metadata or tags"
            );
        }
    }
    let added = sync_links(&tx, id)?;
    tx.execute(
        &format!(
            "UPDATE objects SET revision=revision+1,updated_at={} WHERE id=?1",
            now()
        ),
        [id],
    )?;
    db::save_revision(&tx, id, actor)?;
    let verb = match op {
        "archive" => "archived",
        "unarchive" => "restored",
        _ => "edited",
    };
    events::emit(
        &tx,
        actor,
        &format!("{}.{verb}", before["kind"].as_str().unwrap()),
        id,
        &json!({"fields":fields}),
    )?;
    emit_references(&tx, actor, id, &added)?;
    let out = object_output(&tx, id, true)?;
    tx.commit()?;
    Ok(out)
}

fn list(
    conn: &Connection,
    actor: &str,
    args: &Value,
    kind: Option<&str>,
    updates: bool,
) -> Result<Output> {
    let mut predicates = Vec::new();
    let mut binds = Vec::<rusqlite::types::Value>::new();
    if let Some(kind) = kind.or_else(|| text(args, "kind")) {
        if kind == "task" {
            predicates.push("EXISTS(SELECT 1 FROM tasks t WHERE t.object_id=o.id)".into());
        } else {
            predicates.push("o.kind=?".to_string());
            binds.push(kind.to_string().into());
        }
    }
    if !flag(args, "all") {
        predicates.push(format!("{}=?", archive_predicate("o")));
        binds.push(i64::from(flag(args, "archived")).into());
    }
    if let Some(forum) = text(args, "forum") {
        let id = resolve_forum(conn, forum)?;
        if flag(args, "recursive") {
            let column = if kind == Some("forum") {
                "o.parent_id"
            } else {
                "o.forum_id"
            };
            predicates.push(format!("{column} IN (WITH RECURSIVE subtree(id) AS (SELECT ? UNION ALL SELECT f.id FROM objects f JOIN subtree s ON f.parent_id=s.id WHERE f.kind='forum') SELECT id FROM subtree)"));
        } else if kind == Some("forum") {
            predicates.push("o.parent_id=?".into());
        } else {
            predicates.push("o.forum_id=?".into());
        }
        binds.push(id.into());
    }
    if let Some(author) = text(args, "author") {
        predicates.push("o.author=?".into());
        binds.push(author.to_owned().into());
    }
    if let Some(tag) = text(args, "tag") {
        predicates.push("EXISTS(SELECT 1 FROM tags t WHERE t.object_id=o.id AND t.tag=?)".into());
        binds.push(tag.to_owned().into());
    }
    if let Some(post) = args.get("post").and_then(Value::as_i64) {
        require_kind(conn, post, "post")?;
        predicates.push("o.parent_id=?".into());
        binds.push(post.into());
    }
    if updates {
        predicates.push("o.revision>COALESCE((SELECT seen_revision FROM view_state v WHERE v.agent=? AND v.object_id=o.id),0)".into());
        binds.push(actor.to_owned().into());
    }
    let limit = model::limit(args);
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(i64::MAX as u64) as i64;
    let order = if kind == Some("comment") {
        "o.created_at ASC,o.id ASC"
    } else {
        "o.updated_at DESC,o.id DESC"
    };
    let clause = if predicates.is_empty() {
        "1".into()
    } else {
        predicates.join(" AND ")
    };
    let sql =
        format!("SELECT o.id FROM objects o WHERE {clause} ORDER BY {order} LIMIT ? OFFSET ?");
    binds.push(((limit + 1) as i64).into());
    binds.push(offset.into());
    let ids: Vec<i64> = conn
        .prepare(&sql)?
        .query_map(params_from_iter(binds), |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let full = flag(args, "full") || (kind == Some("comment") && !flag(args, "compact"));
    let mut out = Output {
        kind: if updates { "updates" } else { "objects" }.into(),
        more: ids.len() > limit,
        ..Output::default()
    };
    for id in ids.into_iter().take(limit) {
        let (mut item, receipt) = present(db::get_object(conn, id)?, full);
        if updates {
            let changes = net_changes(conn, actor, id, None, None)?;
            item["changed_fields"] = json!(
                changes["fields"]
                    .as_object()
                    .map(|m| m.keys().collect::<Vec<_>>())
                    .unwrap_or_default()
            );
            if full && changes["from_revision"].as_i64().unwrap_or(0) > 0 {
                item.as_object_mut().unwrap().remove("body");
                item["body_omitted"] = json!(true);
                item["patch"] = json!(change_patch(id, &changes));
                item["changes"] = changes;
            }
        }
        out.items.push(item);
        out.receipts.push(receipt);
    }
    if !full && !out.items.is_empty() {
        out.notices
            .push("Bodies omitted; use --full or post/comment/forum show <id>.".into());
    }
    Ok(out)
}

fn snapshot(conn: &Connection, id: i64, revision: i64) -> Result<Value> {
    let raw: String = conn
        .query_row(
            "SELECT snapshot FROM revisions WHERE object_id=?1 AND revision=?2",
            params![id, revision],
            |r| r.get(0),
        )
        .optional()?
        .with_context(|| format!("revision {revision} does not exist for #{id}"))?;
    Ok(serde_json::from_str(&raw)?)
}

fn net_changes(
    conn: &Connection,
    actor: &str,
    id: i64,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<Value> {
    let current = db::get_object(conn, id)?;
    let to = to.unwrap_or(current["revision"].as_i64().context("missing revision")?);
    let from = match from {
        Some(n) => n,
        None => conn
            .query_row(
                "SELECT read_revision FROM view_state WHERE agent=?1 AND object_id=?2",
                params![actor, id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0),
    };
    ensure!(from >= 0 && to >= from, "diff requires 0 <= from <= to");
    let old = if from == 0 {
        json!({})
    } else {
        snapshot(conn, id, from)?
    };
    let new = snapshot(conn, id, to)?;
    let mut changes = serde_json::Map::new();
    for key in [
        "title",
        "body",
        "summary",
        "metadata",
        "tags",
        "archived",
        "task",
        "dependencies",
        "links",
    ] {
        let mut before = old.get(key).cloned();
        let mut after = new.get(key).cloned();
        if key == "task" {
            // Task timestamps belong in history; a claim followed by release has
            // no semantic net change when owner, state and dependencies agree.
            for value in [&mut before, &mut after] {
                if let Some(Value::Object(task)) = value {
                    task.remove("updated_at");
                }
            }
        }
        if before != after {
            changes.insert(key.into(), json!({"before":before,"after":after}));
        }
    }
    Ok(json!({"from_revision":from,"to_revision":to,"initial":from==0,"fields":changes}))
}

fn history(conn: &Connection, args: &Value) -> Result<Output> {
    let id = target(conn, args)?;
    db::get_object(conn, id)?;
    let limit = model::limit(args);
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(i64::MAX as u64) as i64;
    let mut items:Vec<Value>=conn.prepare("SELECT revision,author,created_at,snapshot FROM revisions WHERE object_id=?1 ORDER BY revision DESC LIMIT ?2 OFFSET ?3")?.query_map(params![id,(limit+1) as i64,offset],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))?.map(|r|{let(revision,author,at,snapshot)=r?;let mut item=json!({"id":id,"revision":revision,"author":author,"created_at":at});if flag(args,"full"){item["snapshot"]=serde_json::from_str(&snapshot)?;}Ok(item)}).collect::<Result<_>>()?;
    let more = items.len() > limit;
    items.truncate(limit);
    Ok(Output {
        kind: "history".into(),
        items,
        more,
        ..Output::default()
    })
}

fn linked(conn: &Connection, args: &Value, back: bool) -> Result<Output> {
    let id = target(conn, args)?;
    db::get_object(conn, id)?;
    let limit = model::limit(args);
    let sql = if back {
        "SELECT source_id FROM links WHERE target_id=?1 ORDER BY source_id LIMIT ?2 OFFSET ?3"
    } else {
        "SELECT target_id FROM links WHERE source_id=?1 ORDER BY target_id LIMIT ?2 OFFSET ?3"
    };
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(i64::MAX as u64) as i64;
    let ids: Vec<i64> = conn
        .prepare(sql)?
        .query_map(params![id, (limit + 1) as i64, offset], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Output {
        kind: "objects".into(),
        more: ids.len() > limit,
        ..Output::default()
    };
    for id in ids.into_iter().take(limit) {
        let (item, receipt) = present(db::get_object(conn, id)?, flag(args, "full"));
        out.items.push(item);
        out.receipts.push(receipt);
    }
    Ok(out)
}

fn thread(conn: &Connection, args: &Value) -> Result<Output> {
    let id = target(conn, args)?;
    // Determine the containing post without reading a body that may not belong
    // to the requested page.
    let (kind, parent): (String, Option<i64>) = conn
        .query_row(
            "SELECT kind,parent_id FROM objects WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .with_context(|| format!("object #{id} does not exist"))?;
    ensure!(
        kind == "post" || kind == "comment",
        "thread requires a post or comment ID"
    );
    let post = if kind == "post" {
        id
    } else {
        parent.context("comment has no post")?
    };
    let tree = flag(args, "tree") || kind == "comment";
    let limit = model::limit(args);
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(i64::MAX as u64) as i64;
    let visibility = if flag(args, "all") {
        "1".to_owned()
    } else {
        format!("NOT ({})", archive_predicate("o"))
    };
    // Entries contain only (object/event ID, system-event flag, tree depth).
    // Hydrate bodies and event details only after selecting the requested page.
    let entries: Vec<(i64, bool, usize)> = if !tree {
        let sql = format!(
            "SELECT id,system FROM (
            SELECT o.id,0 AS system,o.created_at FROM objects o
            WHERE (o.id=?1 OR (o.kind='comment' AND o.parent_id=?1)) AND {visibility}
            UNION ALL
            SELECT e.id,1 AS system,e.created_at FROM events e
            WHERE e.post_id=?1 AND e.kind NOT IN ('post.created','comment.created')
            ) ORDER BY created_at,id,system LIMIT ?2 OFFSET ?3"
        );
        conn.prepare(&sql)?
            .query_map(params![post, (limit + 1) as i64, offset], |r| {
                Ok((r.get(0)?, r.get(1)?, 0))
            })?
            .collect::<rusqlite::Result<_>>()?
    } else {
        let sql = if kind == "comment" {
            format!("WITH RECURSIVE replies(id) AS (SELECT ?1 UNION ALL SELECT o.id FROM objects o JOIN replies r ON o.reply_to=r.id WHERE o.kind='comment')
                SELECT o.id,o.reply_to FROM objects o JOIN replies r ON r.id=o.id WHERE {visibility} ORDER BY o.created_at,o.id")
        } else {
            format!(
                "SELECT o.id,o.reply_to FROM objects o WHERE (o.id=?1 OR (o.kind='comment' AND o.parent_id=?1)) AND {visibility} ORDER BY o.created_at,o.id"
            )
        };
        // Tree traversal needs relationships across the discussion, but never
        // loads their bodies, tags, metadata, task state or revision snapshots.
        let relationships: Vec<(i64, Option<i64>)> = conn
            .prepare(&sql)?
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let visible: BTreeSet<i64> = relationships.iter().map(|r| r.0).collect();
        let mut children = std::collections::BTreeMap::<i64, Vec<i64>>::new();
        let mut roots = Vec::new();
        for (child, parent) in &relationships {
            match parent.filter(|parent| visible.contains(parent)) {
                Some(parent) if *child != id => children.entry(parent).or_default().push(*child),
                _ => roots.push(*child),
            }
        }
        let mut stack: Vec<(i64, usize)> = roots.into_iter().rev().map(|id| (id, 0)).collect();
        let mut traversed = 0i64;
        let mut selected = Vec::new();
        while let Some((child, depth)) = stack.pop() {
            if traversed >= offset {
                selected.push((child, false, depth));
            }
            if selected.len() > limit {
                break;
            }
            traversed += 1;
            if let Some(child_ids) = children.get(&child) {
                stack.extend(child_ids.iter().rev().map(|id| (*id, depth + 1)));
            }
        }
        selected
    };
    let mut out = Output {
        kind: "thread".into(),
        more: entries.len() > limit,
        ..Output::default()
    };
    for (id, system, depth) in entries.into_iter().take(limit) {
        if system {
            let (kind, detail, at): (String, String, String) = conn.query_row(
                "SELECT kind,detail,created_at FROM events WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            out.items.push(json!({"kind":"system","event_id":id,"event_kind":kind,"detail":serde_json::from_str::<Value>(&detail)?,"created_at":at}));
        } else {
            let mut item = db::get_object(conn, id)?;
            if tree {
                item["depth"] = json!(depth);
            }
            let (item, receipt) = present(item, !flag(args, "compact"));
            out.items.push(item);
            out.receipts.push(receipt);
        }
    }
    Ok(out)
}

pub fn execute(conn: &mut Connection, actor: &str, req: &Request) -> Result<Output> {
    let (family, op) = req.command.split_once('.').unwrap_or((&req.command, ""));
    match family {
        "forum" | "post" | "comment" => match op {
            "create" => create(conn, actor, req, family),
            "edit" | "archive" | "unarchive" => edit(conn, actor, req, Some(family), op),
            "show" => {
                let id = target(conn, &req.args)?;
                require_kind(conn, id, family)?;
                object_output(conn, id, !flag(&req.args, "compact"))
            }
            "list" => list(conn, actor, &req.args, Some(family), false),
            "history" => history(conn, &req.args),
            "diff" => diff_output(conn, actor, &req.args),
            _ => bail!("unknown content command {}", req.command),
        },
        "tag" => match op {
            "add" | "remove" => edit(conn, actor, req, None, &req.command),
            "list" => {
                let limit = model::limit(&req.args);
                let offset = req
                    .args
                    .get("offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(i64::MAX as u64) as i64;
                let mut rows:Vec<Value> = conn
                    .prepare("SELECT tag,count(*) FROM tags GROUP BY tag ORDER BY tag LIMIT ?1 OFFSET ?2")?
                    .query_map(params![(limit+1) as i64,offset], |r| {
                        Ok(json!({"tag":r.get::<_,String>(0)?,"count":r.get::<_,i64>(1)?}))
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                let more = rows.len() > limit;
                rows.truncate(limit);
                Ok(Output {
                    kind: "tags".into(),
                    items: rows,
                    more,
                    ..Output::default()
                })
            }
            _ => bail!("unknown tag command"),
        },
        "metadata" | "summary" if op == "set" => edit(conn, actor, req, None, "edit"),
        "history" => history(conn, &req.args),
        "diff" => diff_output(conn, actor, &req.args),
        "updates" => list(conn, actor, &req.args, None, true),
        "links" => linked(conn, &req.args, false),
        "backlinks" => linked(conn, &req.args, true),
        "thread" => thread(conn, &req.args),
        _ => bail!("unknown content command {}", req.command),
    }
}

fn change_patch(id: i64, changes: &Value) -> String {
    let mut patch = format!(
        "#{id}: revision {} -> {}\n",
        changes["from_revision"], changes["to_revision"]
    );
    if let Some(fields) = changes["fields"].as_object() {
        for (field, change) in fields {
            if ["title", "body", "summary"].contains(&field.as_str()) {
                let before = change["before"].as_str().unwrap_or("");
                let after = change["after"].as_str().unwrap_or("");
                patch.push_str(
                    &similar::TextDiff::from_lines(before, after)
                        .unified_diff()
                        .header(
                            &format!("{field}@{}", changes["from_revision"]),
                            &format!("{field}@{}", changes["to_revision"]),
                        )
                        .to_string(),
                );
            } else {
                patch.push_str(&format!(
                    "{field}: {} -> {}\n",
                    change["before"], change["after"]
                ));
            }
        }
        if fields.is_empty() {
            patch.push_str("No content changes.\n");
        }
    }
    patch
}

fn diff_output(conn: &Connection, actor: &str, args: &Value) -> Result<Output> {
    let id = target(conn, args)?;
    let changes = net_changes(
        conn,
        actor,
        id,
        args.get("from").and_then(Value::as_i64),
        args.get("to").and_then(Value::as_i64),
    )?;
    let patch = change_patch(id, &changes);
    let mut out = Output::one(
        "diff",
        json!({"id":id,"revision":changes["to_revision"],"changes":changes,"patch":patch}),
    );
    // Only a current full diff supplies all information needed to advance its baseline.
    let revision = changes["to_revision"].as_i64().unwrap_or(0);
    let known: i64 = conn
        .query_row(
            "SELECT read_revision FROM view_state WHERE agent=?1 AND object_id=?2",
            params![actor, id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let baseline = changes["from_revision"].as_i64().unwrap_or(0);
    if revision == db::get_object(conn, id)?["revision"].as_i64().unwrap_or(0)
        && (baseline == 0 || baseline == known)
    {
        out.receipts.push(Receipt {
            object_id: id,
            revision,
            full: true,
        });
    }
    Ok(out)
}
