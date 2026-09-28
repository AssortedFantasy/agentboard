//! Small, read-only HTML inspection server. Browsing never acknowledges agent activity.
use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params};
use std::path::Path;
use tiny_http::{Header, Method, Response, Server, StatusCode};

const PAGE: i64 = 50;
const FIELDS: &str = "id,kind,forum_id,parent_id,reply_to,path,title,body,summary,metadata,archived,revision,author,created_at,updated_at";

struct Object {
    id: i64,
    kind: String,
    forum: Option<i64>,
    parent: Option<i64>,
    reply: Option<i64>,
    path: Option<String>,
    title: String,
    body: String,
    summary: String,
    metadata: String,
    archived: bool,
    revision: i64,
    author: String,
    created: String,
    updated: String,
}

fn object_row(row: &Row<'_>) -> rusqlite::Result<Object> {
    Ok(Object {
        id: row.get(0)?,
        kind: row.get(1)?,
        forum: row.get(2)?,
        parent: row.get(3)?,
        reply: row.get(4)?,
        path: row.get(5)?,
        title: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        body: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        summary: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        metadata: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        archived: row.get(10)?,
        revision: row.get(11)?,
        author: row.get(12)?,
        created: row.get(13)?,
        updated: row.get(14)?,
    })
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let high = (bytes[i + 1] as char).to_digit(16)?;
            let low = (bytes[i + 2] as char).to_digit(16)?;
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn link(id: i64) -> String {
    format!("<a href=\"/objects/{id}\">#{id}</a>")
}
fn agent_link(name: &str) -> String {
    format!("<a href=\"/agents/{}\">{}</a>", encode(name), escape(name))
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{} — Agentboard</title><style>body{{font:16px/1.5 system-ui,sans-serif;max-width:70rem;margin:2rem auto;padding:0 1rem}}nav a{{margin-right:1rem}}article{{border:1px solid #999;padding:1rem;margin:1rem 0}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;font:inherit}}.meta{{font-size:.9rem;color:#555}}table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #aaa;padding:.4rem;text-align:left;overflow-wrap:anywhere}}h1,h2,h3{{line-height:1.2}}code{{overflow-wrap:anywhere}}</style></head><body><nav><a href=\"/\">Posts</a><a href=\"/forums\">Forums</a><a href=\"/tasks\">Tasks</a><a href=\"/tags\">Tags</a><a href=\"/agents\">Agents</a><a href=\"/activity\">Activity</a><a href=\"/archive\">Archive</a></nav><p class=\"meta\">Read-only inspection. Browsing does not change agent read state or notifications.</p><main><h1>{}</h1>{body}</main></body></html>",
        escape(title),
        escape(title)
    )
}

fn pagination(path: &str, offset: i64, more: bool) -> String {
    let mut html = String::from("<p>");
    if offset > 0 {
        html.push_str(&format!(
            "<a href=\"{}?offset={}\">Previous</a> ",
            escape(path),
            (offset - PAGE).max(0)
        ));
    }
    if more {
        html.push_str(&format!(
            "<a href=\"{}?offset={}\">Next (more results)</a>",
            escape(path),
            offset + PAGE
        ));
    }
    html.push_str("</p>");
    html
}

fn card(conn: &Connection, item: &Object, full: bool) -> Result<String> {
    let inherited_archive = !item.archived && crate::content::effectively_archived(conn, item.id)?;
    let heading = if item.title.is_empty() {
        &item.kind
    } else {
        &item.title
    };
    let mut out = format!(
        "<article id=\"object-{}\"><h2>{} {}</h2><p class=\"meta\">{} · {} · revision {} · created {} · updated {}{}</p>",
        item.id,
        link(item.id),
        escape(heading),
        escape(&item.kind),
        agent_link(&item.author),
        item.revision,
        escape(&item.created),
        escape(&item.updated),
        if item.archived {
            " · archived"
        } else if inherited_archive {
            " · archived through parent"
        } else {
            ""
        }
    );
    if let Some(path) = &item.path {
        out.push_str(&format!("<p>Forum path: {}</p>", escape(path)));
    }
    for (label, id) in [
        ("Forum", item.forum),
        ("Parent", item.parent),
        ("Reply to", item.reply),
    ] {
        if let Some(id) = id {
            out.push_str(&format!("<span>{label}: {} </span>", link(id)));
        }
    }
    let task = conn
        .query_row(
            "SELECT status,owner FROM tasks WHERE object_id=?1",
            [item.id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    if let Some((status, owner)) = task {
        out.push_str(&format!(
            "<p>Task: <strong>{}</strong> · owner: {}</p>",
            escape(&status),
            owner
                .as_deref()
                .map(agent_link)
                .unwrap_or_else(|| "unassigned".into())
        ));
        for (label, sql) in [
            (
                "Depends on",
                "SELECT prerequisite_id FROM dependencies WHERE task_id=?1 ORDER BY prerequisite_id LIMIT 51",
            ),
            (
                "Blocks",
                "SELECT task_id FROM dependencies WHERE prerequisite_id=?1 ORDER BY task_id LIMIT 51",
            ),
        ] {
            out.push_str(&related(conn, label, sql, item.id)?);
        }
    }
    let mut stmt = conn.prepare("SELECT tag FROM tags WHERE object_id=?1 ORDER BY tag LIMIT 51")?;
    let tags = stmt
        .query_map([item.id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !tags.is_empty() {
        out.push_str("<p>Tags: ");
        for tag in tags.iter().take(PAGE as usize) {
            out.push_str(&format!(
                "<a href=\"/tags/{}\">{}</a> ",
                encode(tag),
                escape(tag)
            ));
        }
        if tags.len() > PAGE as usize {
            out.push_str("(additional tags omitted)");
        }
        out.push_str("</p>");
    }
    if !item.summary.is_empty() {
        out.push_str(&format!(
            "<h3>Summary</h3><pre>{}</pre>",
            escape(&item.summary)
        ));
    }
    if full {
        out.push_str(&format!("<pre>{}</pre>", escape(&item.body)));
        if !item.metadata.is_empty() && item.metadata != "{}" {
            out.push_str(&format!(
                "<details><summary>Metadata</summary><pre>{}</pre></details>",
                escape(&item.metadata)
            ));
        }
        out.push_str(&related(
            conn,
            "Mentions",
            "SELECT target_id FROM links WHERE source_id=?1 ORDER BY target_id LIMIT 51",
            item.id,
        )?);
        out.push_str(&related(
            conn,
            "Mentioned in",
            "SELECT source_id FROM links WHERE target_id=?1 ORDER BY source_id LIMIT 51",
            item.id,
        )?);
        out.push_str(&format!(
            "<p><a href=\"/history/{}\">Revision history</a></p>",
            item.id
        ));
    }
    out.push_str("</article>");
    Ok(out)
}

fn related(conn: &Connection, label: &str, sql: &str, id: i64) -> Result<String> {
    let mut stmt = conn.prepare(sql)?;
    let ids = stmt
        .query_map([id], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if ids.is_empty() {
        return Ok(String::new());
    }
    let mut out = format!("<p>{label}: ");
    for id in ids.iter().take(PAGE as usize) {
        out.push_str(&format!("{} ", link(*id)));
    }
    if ids.len() > PAGE as usize {
        out.push_str("(additional links omitted)");
    }
    out.push_str("</p>");
    Ok(out)
}

fn objects(
    conn: &Connection,
    filter: &str,
    parameter: &str,
    offset: i64,
    ascending: bool,
) -> Result<(String, bool)> {
    let order = if ascending { "ASC" } else { "DESC" };
    // All filters below are internal constants. Archive visibility includes ancestors.
    let archive = crate::content::archive_predicate("objects");
    let filter = filter
        .replace("archived=0", &format!("NOT ({archive})"))
        .replace("archived=1", &archive);
    // Filter/order are internal constants; user values are always bound parameters.
    let sql = format!(
        "SELECT {FIELDS} FROM objects WHERE {filter} ORDER BY id {order} LIMIT ?2 OFFSET ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![parameter, PAGE + 1, offset], object_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut html = String::new();
    for item in rows.iter().take(PAGE as usize) {
        html.push_str(&card(conn, item, item.kind == "comment")?);
    }
    if rows.is_empty() {
        html.push_str("<p>No results.</p>");
    }
    Ok((html, rows.len() > PAGE as usize))
}

fn discussion(conn: &Connection, post: i64, offset: i64) -> Result<(String, bool)> {
    let mut stmt = conn.prepare("SELECT source,id FROM (SELECT 'comment' AS source,id,created_at FROM objects WHERE kind='comment' AND parent_id=?1 UNION ALL SELECT 'event',id,created_at FROM events WHERE post_id=?1 AND kind NOT IN ('comment.created','post.created','task.created')) ORDER BY created_at,id,source LIMIT ?2 OFFSET ?3")?;
    let rows = stmt
        .query_map(params![post, PAGE + 1, offset], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut html = String::new();
    for (source, id) in rows.iter().take(PAGE as usize) {
        if source == "comment" {
            let item = conn.query_row(
                &format!("SELECT {FIELDS} FROM objects WHERE id=?1"),
                [id],
                object_row,
            )?;
            html.push_str(&card(conn, &item, true)?);
        } else {
            let (actor, kind, time, detail) = conn.query_row(
                "SELECT actor,kind,created_at,detail FROM events WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )?;
            html.push_str(&format!(
                "<article><h3>System: {}</h3><p class=\"meta\">{} · {}</p><pre>{}</pre></article>",
                escape(&kind),
                agent_link(&actor),
                escape(&time),
                escape(&detail)
            ));
        }
    }
    if rows.is_empty() {
        html.push_str("<p>No discussion yet.</p>");
    }
    Ok((html, rows.len() > PAGE as usize))
}

/// Renders bounded pages without changing the supplied database or attention state.
pub fn render_route(conn: &Connection, url: &str) -> Result<(u16, String)> {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let offset = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("offset="))
        .unwrap_or("0");
    let Ok(offset) = offset.parse::<i64>() else {
        return Ok((
            400,
            page(
                "Invalid page",
                "<p>offset must be a nonnegative integer.</p>",
            ),
        ));
    };
    if !(0..=1_000_000).contains(&offset) {
        return Ok((
            400,
            page(
                "Invalid page",
                "<p>offset must be between 0 and 1000000.</p>",
            ),
        ));
    }
    let (title, mut html, more) = match path {
        "/" | "/forums" | "/tasks" | "/archive" => {
            let (title, filter) = match path {
                "/forums" => ("Forums", "kind='forum' AND archived=0 AND ?1 IS NOT NULL"),
                "/tasks" => (
                    "Tasks",
                    "id IN (SELECT object_id FROM tasks) AND archived=0 AND ?1 IS NOT NULL",
                ),
                "/archive" => ("Archive", "archived=1 AND ?1 IS NOT NULL"),
                _ => ("Posts", "kind='post' AND archived=0 AND ?1 IS NOT NULL"),
            };
            let (html, more) = objects(conn, filter, "", offset, path == "/forums")?;
            (title.to_string(), html, more)
        }
        "/tags" => {
            let mut stmt = conn.prepare(&format!("SELECT tag,COUNT(*) FROM tags JOIN objects ON objects.id=tags.object_id WHERE NOT ({}) GROUP BY tag ORDER BY tag LIMIT ?1 OFFSET ?2", crate::content::archive_predicate("objects")))?;
            let rows = stmt
                .query_map(params![PAGE + 1, offset], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut html = String::from("<ul>");
            for (tag, count) in rows.iter().take(PAGE as usize) {
                html.push_str(&format!(
                    "<li><a href=\"/tags/{}\">{}</a> ({count})</li>",
                    encode(tag),
                    escape(tag)
                ));
            }
            html.push_str("</ul>");
            ("Tags".into(), html, rows.len() > PAGE as usize)
        }
        "/agents" => {
            let mut stmt = conn
                .prepare("SELECT name,last_active FROM agents ORDER BY name LIMIT ?1 OFFSET ?2")?;
            let rows = stmt
                .query_map(params![PAGE + 1, offset], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut html = String::from("<ul>");
            for (name, active) in rows.iter().take(PAGE as usize) {
                html.push_str(&format!(
                    "<li>{} · last active {}</li>",
                    agent_link(name),
                    escape(active)
                ));
            }
            html.push_str("</ul>");
            ("Agents".into(), html, rows.len() > PAGE as usize)
        }
        "/activity" => {
            let mut stmt = conn.prepare("SELECT id,actor,kind,object_id,created_at,detail FROM events ORDER BY id DESC LIMIT ?1 OFFSET ?2")?;
            let rows = stmt
                .query_map(params![PAGE + 1, offset], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut html = String::new();
            for (id, actor, kind, object, time, detail) in rows.iter().take(PAGE as usize) {
                html.push_str(&format!(
                    "<article><h2>Event {id}: {}</h2><p>{} · {} · {}</p><pre>{}</pre></article>",
                    escape(kind),
                    agent_link(actor),
                    object.map(link).unwrap_or_default(),
                    escape(time),
                    escape(detail)
                ));
            }
            ("Activity".into(), html, rows.len() > PAGE as usize)
        }
        _ if path.starts_with("/objects/") => {
            let Ok(id) = path[9..].parse::<i64>() else {
                return Ok(not_found());
            };
            let item = conn
                .query_row(
                    &format!("SELECT {FIELDS} FROM objects WHERE id=?1"),
                    [id],
                    object_row,
                )
                .optional()?;
            let Some(item) = item else {
                return Ok(not_found());
            };
            let mut html = card(conn, &item, true)?;
            let mut more = false;
            if item.kind == "forum" {
                let filter = if crate::content::effectively_archived(conn, id)? {
                    "((kind='forum' AND parent_id=?1) OR (kind='post' AND forum_id=?1))"
                } else {
                    "((kind='forum' AND parent_id=?1) OR (kind='post' AND forum_id=?1)) AND archived=0"
                };
                let (children, omitted) = objects(conn, filter, &id.to_string(), offset, true)?;
                html.push_str("<h2>Contents</h2>");
                html.push_str(&children);
                more = omitted;
            } else if item.kind == "post" {
                let (comments, omitted) = discussion(conn, id, offset)?;
                html.push_str("<h2>Discussion (chronological)</h2>");
                html.push_str(&comments);
                more = omitted;
            }
            (format!("#{} {}", id, item.title), html, more)
        }
        _ if path.starts_with("/tags/") => {
            let Some(tag) = decode(&path[6..]) else {
                return Ok(not_found());
            };
            let (html, more) = objects(
                conn,
                "id IN (SELECT object_id FROM tags WHERE tag=?1) AND archived=0",
                &tag,
                offset,
                false,
            )?;
            (format!("Tag: {tag}"), html, more)
        }
        _ if path.starts_with("/agents/") => {
            let Some(name) = decode(&path[8..]) else {
                return Ok(not_found());
            };
            let agent = conn
                .query_row(
                    "SELECT profile,created_at,last_active FROM agents WHERE name=?1",
                    [&name],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;
            let Some((profile, created, active)) = agent else {
                return Ok(not_found());
            };
            let mut html = format!(
                "<p>Created {} · last active {}</p><h2>Profile</h2><pre>{}</pre><h2>Authored content</h2>",
                escape(&created),
                escape(&active),
                escape(&profile)
            );
            let (content, more) = objects(conn, "author=?1", &name, offset, false)?;
            html.push_str(&content);
            (format!("Agent: {name}"), html, more)
        }
        _ if path.starts_with("/history/") => {
            let Ok(id) = path[9..].parse::<i64>() else {
                return Ok(not_found());
            };
            if !conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM objects WHERE id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )? {
                return Ok(not_found());
            }
            let mut stmt = conn.prepare("SELECT revision,snapshot,author,created_at FROM revisions WHERE object_id=?1 ORDER BY revision DESC LIMIT ?2 OFFSET ?3")?;
            let rows = stmt
                .query_map(params![id, PAGE + 1, offset], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut html = format!("<p>Current object: {}</p>", link(id));
            for (revision, snapshot, author, time) in rows.iter().take(PAGE as usize) {
                let pretty = serde_json::from_str::<serde_json::Value>(snapshot)
                    .ok()
                    .and_then(|v| serde_json::to_string_pretty(&v).ok())
                    .unwrap_or_else(|| snapshot.clone());
                html.push_str(&format!(
                    "<article><h2>Revision {revision}</h2><p>{} · {}</p><pre>{}</pre></article>",
                    agent_link(author),
                    escape(time),
                    escape(&pretty)
                ));
            }
            (
                format!("History of #{id}"),
                html,
                rows.len() > PAGE as usize,
            )
        }
        _ => return Ok(not_found()),
    };
    html.push_str(&pagination(path, offset, more));
    Ok((200, page(&title, &html)))
}

fn not_found() -> (u16, String) {
    (404, page("Not found", "<p>No such page or object.</p>"))
}

/// Opens an existing board read-only and serves GET/HEAD until the process is stopped.
pub fn serve(database: &Path, bind: &str) -> Result<()> {
    serve_inner(database, bind, None)
}

/// CLI entrypoint: records the launch once, then handles all HTTP requests read-only.
pub fn serve_as(database: &Path, bind: &str, actor: &str) -> Result<()> {
    serve_inner(database, bind, Some(actor))
}

fn log_launch(
    database: &Path,
    bind: &str,
    actor: Option<&str>,
    error: Option<&str>,
    duration_ms: i64,
) {
    let Some(actor) = actor else {
        return;
    };
    let logged = (|| -> Result<()> {
        let conn = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let request = crate::model::Request {
            command: "serve".into(),
            args: serde_json::json!({"bind":bind}),
        };
        let result = match error {
            Some(error) => Err(anyhow!("{error}")),
            None => Ok(crate::model::Output::default()),
        };
        crate::app::log_command(&conn, actor, &request, &result, duration_ms)
    })();
    if let Err(error) = logged {
        eprintln!("warning: could not persist server launch log: {error:#}");
    }
}

fn serve_inner(database: &Path, bind: &str, actor: Option<&str>) -> Result<()> {
    let started = std::time::Instant::now();
    let conn = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open existing board {} read-only", database.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "query_only", true)?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != crate::db::SCHEMA_VERSION {
        anyhow::bail!(
            "board schema {version} does not match supported schema {}; initialize or migrate the board with a compatible Agentboard CLI before serving",
            crate::db::SCHEMA_VERSION
        );
    }
    let server = match Server::http(bind) {
        Ok(server) => server,
        Err(error) => {
            let error = anyhow!("bind {bind}: {error}");
            log_launch(
                database,
                bind,
                actor,
                Some(&error.to_string()),
                started.elapsed().as_millis().min(i64::MAX as u128) as i64,
            );
            return Err(error);
        }
    };
    log_launch(
        database,
        bind,
        actor,
        None,
        started.elapsed().as_millis().min(i64::MAX as u128) as i64,
    );
    eprintln!(
        "Agentboard read-only web interface: http://{}",
        server.server_addr()
    );
    for request in server.incoming_requests() {
        let head = request.method() == &Method::Head;
        let (status, html) = if request.method() != &Method::Get && !head {
            (
                405,
                page(
                    "Method not allowed",
                    "<p>This server supports GET and HEAD only.</p>",
                ),
            )
        } else {
            match render_route(&conn, request.url()) {
                Ok(response) => response,
                Err(error) => {
                    eprintln!("Web request failed: {error:#}");
                    (
                        500,
                        page(
                            "Unable to render page",
                            "<p>See the server terminal for details.</p>",
                        ),
                    )
                }
            }
        };
        let mut response = Response::from_string(if head { String::new() } else { html })
            .with_status_code(StatusCode(status));
        for (name, value) in [
            ("Content-Type", "text/html; charset=utf-8"),
            ("Cache-Control", "no-store"),
            ("X-Content-Type-Options", "nosniff"),
            (
                "Content-Security-Policy",
                "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'",
            ),
        ] {
            response.add_header(Header::from_bytes(name, value).expect("static valid HTTP header"));
        }
        if status == 405 {
            response.add_header(
                Header::from_bytes("Allow", "GET, HEAD").expect("static valid HTTP header"),
            );
        }
        if let Err(error) = request.respond(response) {
            eprintln!("Web response failed: {error}");
        }
    }
    Ok(())
}
