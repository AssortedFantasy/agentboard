use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

pub const SCHEMA_VERSION: i64 = 1;

pub fn open(path: impl AsRef<Path>) -> Result<Connection> {
    let conn = Connection::open(path).context("open board database")?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION { bail!("board schema {version} is newer than supported schema {SCHEMA_VERSION}; upgrade agentboard"); }
    if version == SCHEMA_VERSION { return Ok(()); }
    conn.execute_batch("BEGIN IMMEDIATE")?;
    // Another initializer may have completed while this connection waited.
    let locked_version: i64 = conn.pragma_query_value(None,"user_version",|r|r.get(0))?;
    if locked_version == SCHEMA_VERSION { conn.execute_batch("COMMIT")?; return Ok(()); }
    let result = conn.execute_batch(include_str!("schema.sql"));
    if let Err(error) = result {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(error.into());
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    conn.execute_batch("COMMIT")?;
    Ok(())
}

pub fn ensure_agent(conn: &Connection, actor: &str) -> Result<()> {
    if actor.is_empty() || actor.len() > 128 || actor.chars().any(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))) {
        bail!("agent name must be 1–128 letters, digits, hyphens, underscores or dots");
    }
    conn.execute("INSERT INTO agents(name) VALUES(?1) ON CONFLICT(name) DO UPDATE SET last_active=strftime('%Y-%m-%dT%H:%M:%fZ','now')", [actor])?;
    Ok(())
}

pub fn get_object(conn: &Connection, object_id: i64) -> Result<Value> {
    let mut value = conn.query_row(
        "SELECT id,kind,forum_id,parent_id,reply_to,path,title,body,summary,metadata,archived,revision,author,created_at,updated_at FROM objects WHERE id=?1",
        [object_id], |r| {
            let metadata: String = r.get(9)?;
            Ok(json!({"id":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"forum_id":r.get::<_,Option<i64>>(2)?,"parent_id":r.get::<_,Option<i64>>(3)?,"reply_to":r.get::<_,Option<i64>>(4)?,"path":r.get::<_,Option<String>>(5)?,"title":r.get::<_,String>(6)?,"body":r.get::<_,String>(7)?,"summary":r.get::<_,String>(8)?,"metadata":serde_json::from_str::<Value>(&metadata).unwrap_or(json!({})),"archived":r.get::<_,bool>(10)?,"revision":r.get::<_,i64>(11)?,"author":r.get::<_,String>(12)?,"created_at":r.get::<_,String>(13)?,"updated_at":r.get::<_,String>(14)?}))
        }).optional()?.ok_or_else(|| anyhow::anyhow!("object #{object_id} does not exist"))?;
    let tags = conn.prepare("SELECT tag FROM tags WHERE object_id=?1 ORDER BY tag")?
        .query_map([object_id], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    value["tags"] = json!(tags);
    let links = conn.prepare("SELECT target_id FROM links WHERE source_id=?1 ORDER BY target_id")?
        .query_map([object_id], |r| r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    value["mentions"] = json!(links);
    if let Some(forum_id) = value["forum_id"].as_i64() {
        value["forum"] = json!(conn.query_row("SELECT path FROM objects WHERE id=?1", [forum_id], |r| r.get::<_,String>(0))?);
    }
    let task = conn.query_row("SELECT status,owner,updated_at FROM tasks WHERE object_id=?1", [object_id], |r| Ok(json!({"status":r.get::<_,String>(0)?,"owner":r.get::<_,Option<String>>(1)?,"updated_at":r.get::<_,String>(2)?}))).optional()?;
    if let Some(mut task) = task {
        let deps = conn.prepare("SELECT prerequisite_id FROM dependencies WHERE task_id=?1 ORDER BY prerequisite_id")?.query_map([object_id], |r| r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let blocks = conn.prepare("SELECT task_id FROM dependencies WHERE prerequisite_id=?1 ORDER BY task_id")?.query_map([object_id], |r| r.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        task["depends_on"] = json!(deps);
        task["blocks"] = json!(blocks);
        value["task"] = task;
    }
    Ok(value)
}

pub fn save_revision(conn: &Connection, object_id: i64, actor: &str) -> Result<()> {
    let object = get_object(conn, object_id)?;
    conn.execute("INSERT INTO revisions(object_id,revision,snapshot,author) VALUES(?1,?2,?3,?4)", params![object_id,object["revision"].as_i64(),object.to_string(),actor])?;
    Ok(())
}

pub fn config(conn: &Connection, key: &str) -> Result<Option<Value>> {
    let raw: Option<String> = conn.query_row("SELECT value FROM config WHERE key=?1", [key], |r| r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).context("invalid stored configuration")).transpose()
}
