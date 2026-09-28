//! Durable attention routing. Retrieval returns receipts; only the output layer acknowledges them.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::model::{Output, Request};

struct EventObject {
    kind: String,
    forum_id: Option<i64>,
    parent_id: Option<i64>,
    reply_to: Option<i64>,
    title: String,
    body: String,
    revision: i64,
}

fn auto_subscribe(conn: &Connection, agent: &str, post: i64) -> Result<()> {
    conn.execute("INSERT OR IGNORE INTO subscriptions(agent,target_type,target,automatic,enabled,inbox) VALUES (?1,'post',?2,1,1,0)", params![agent, post.to_string()])?;
    Ok(())
}

fn mentions(text: &str) -> BTreeSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = BTreeSet::new();
    for (i, ch) in chars.iter().enumerate() {
        if *ch != '@'
            || (i > 0 && (chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '_' | '@')))
        {
            continue;
        }
        let name: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        let name = name.trim_end_matches('.');
        if !name.is_empty() && name.len() <= 128 {
            found.insert(name.to_owned());
        }
    }
    found
}

fn route(
    targets: &mut BTreeMap<String, (BTreeSet<String>, bool)>,
    agent: String,
    reason: String,
    inbox: bool,
) {
    let entry = targets.entry(agent).or_default();
    entry.0.insert(reason);
    entry.1 |= inbox;
}

/// Publish exactly once inside the caller's transaction, after storing the final object state.
pub fn emit(
    conn: &Connection,
    actor: &str,
    kind: &str,
    object_id: i64,
    detail: &Value,
) -> Result<i64> {
    let object: Option<EventObject> = conn
        .query_row(
            "SELECT kind,forum_id,parent_id,reply_to,title,body,revision FROM objects WHERE id=?1",
            [object_id],
            |r| {
                Ok(EventObject {
                    kind: r.get(0)?,
                    forum_id: r.get(1)?,
                    parent_id: r.get(2)?,
                    reply_to: r.get(3)?,
                    title: r.get(4)?,
                    body: r.get(5)?,
                    revision: r.get(6)?,
                })
            },
        )
        .optional()?;
    let post = object.as_ref().and_then(|o| match o.kind.as_str() {
        "post" => Some(object_id),
        "comment" => o.parent_id,
        _ => None,
    });
    conn.execute(
        "INSERT INTO events(actor,kind,object_id,post_id,detail) VALUES (?1,?2,?3,?4,?5)",
        params![actor, kind, object_id, post, detail.to_string()],
    )?;
    let event = conn.last_insert_rowid();
    let mut targets = BTreeMap::new();
    let created = kind.ends_with("created") || kind.ends_with("create");
    let edited = kind.ends_with("edited") || kind.ends_with("edit");
    let previous_object = if edited && let Some(ref o) = object {
        let snapshot: Option<String> = conn.query_row("SELECT snapshot FROM revisions WHERE object_id=?1 AND revision<?2 ORDER BY revision DESC LIMIT 1",params![object_id,o.revision],|r|r.get(0)).optional()?;
        snapshot.and_then(|s| serde_json::from_str::<Value>(&s).ok())
    } else {
        None
    };
    if let Some(post_id) = post {
        if created {
            auto_subscribe(conn, actor, post_id)?;
        }
        let owner: Option<String> = conn
            .query_row(
                "SELECT owner FROM tasks WHERE object_id=?1",
                [post_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(owner) = owner {
            auto_subscribe(conn, &owner, post_id)?;
            if kind.starts_with("task.")
                && [
                    "created",
                    "create",
                    "attached",
                    "attach",
                    "assigned",
                    "assign",
                    "claimed",
                    "claim",
                    "taken_over",
                    "takeover",
                ]
                .iter()
                .any(|suffix| kind.ends_with(suffix))
            {
                route(&mut targets, owner, "assignment".into(), true);
            }
        }
    }
    if let Some(ref o) = object {
        if created || edited {
            let current = mentions(&format!("{}\n{}", o.title, o.body));
            let previous = previous_object
                .as_ref()
                .map(|v| {
                    mentions(&format!(
                        "{}\n{}",
                        v["title"].as_str().unwrap_or(""),
                        v["body"].as_str().unwrap_or("")
                    ))
                })
                .unwrap_or_default();
            for agent in current.difference(&previous) {
                route(&mut targets, agent.clone(), "mention".into(), true);
            }
        }
        if created
            && o.kind == "comment"
            && let Some(parent) = o.reply_to.or(post)
            && let Some(author) = conn
                .query_row("SELECT author FROM objects WHERE id=?1", [parent], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
        {
            route(&mut targets, author, "reply".into(), true);
        }
    }
    let forum_id = object.as_ref().and_then(|o| {
        if o.kind == "forum" {
            Some(object_id)
        } else {
            o.forum_id
        }
    });
    let mut forum_targets = BTreeSet::new();
    let mut pending_forums: Vec<i64> = forum_id
        .into_iter()
        .chain(
            previous_object
                .as_ref()
                .and_then(|v| v["forum_id"].as_i64()),
        )
        .collect();
    let mut current_forum = pending_forums.pop();
    while let Some(id) = current_forum {
        let row: Option<(Option<i64>, String)> = conn
            .query_row(
                "SELECT parent_id,path FROM objects WHERE id=?1 AND kind='forum'",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if !forum_targets.insert(id.to_string()) {
            current_forum = pending_forums.pop();
            continue;
        }
        if let Some((parent, path)) = row {
            forum_targets.insert(path);
            current_forum = parent.or_else(|| pending_forums.pop());
        } else {
            current_forum = pending_forums.pop();
        }
    }
    let mut tags: BTreeSet<String> = {
        let mut statement =
            conn.prepare("SELECT tag FROM tags WHERE object_id=?1 OR object_id=?2")?;
        statement
            .query_map(params![object_id, post], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    if let Some(old_tags) = previous_object.as_ref().and_then(|v| v["tags"].as_array()) {
        tags.extend(old_tags.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    {
        let mut statement = conn
            .prepare("SELECT agent,target_type,target,inbox FROM subscriptions WHERE enabled=1")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })?;
        for row in rows {
            let (agent, typ, target, inbox) = row?;
            let matched = match typ.as_str() {
                "post" => post.is_some_and(|p| p.to_string() == target),
                "forum" => forum_targets.contains(&target),
                "tag" => tags.contains(&target),
                "agent" => target == actor,
                _ => false,
            };
            if matched {
                route(
                    &mut targets,
                    agent,
                    format!("subscription:{typ}:{target}"),
                    inbox,
                );
            }
        }
    }
    if kind.starts_with("task.") {
        let mut statement = conn.prepare("SELECT d.task_id,t.owner FROM dependencies d JOIN tasks t ON t.object_id=d.task_id WHERE d.prerequisite_id=?1")?;
        let rows = statement.query_map([object_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (dependent, owner) = row?;
            let reason = format!("dependency:{dependent}");
            if let Some(owner) = owner {
                route(&mut targets, owner, reason.clone(), true);
            }
            let mut subs = conn.prepare("SELECT agent,inbox FROM subscriptions WHERE target_type='post' AND target=?1 AND enabled=1")?;
            for sub in subs.query_map([dependent.to_string()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
            })? {
                let (agent, inbox) = sub?;
                route(&mut targets, agent, reason.clone(), inbox);
            }
        }
    }
    targets.remove(actor);
    for (agent, (reasons, inbox)) in targets {
        // Names are externally supplied: a mention can await an agent's first invocation.
        conn.execute("INSERT OR IGNORE INTO agents(name) VALUES (?1)", [&agent])?;
        conn.execute(
            "INSERT INTO notifications(agent,event_id,reasons,inbox) VALUES (?1,?2,?3,?4)",
            params![agent, event, serde_json::to_string(&reasons)?, inbox],
        )?;
    }
    Ok(event)
}

fn target(conn: &Connection, args: &Value) -> Result<(String, String)> {
    let typ = crate::model::string(args, "target_type")?;
    let raw = args
        .get("target")
        .ok_or_else(|| anyhow::anyhow!("missing target"))?;
    let value = raw
        .as_str()
        .map(str::to_owned)
        .or_else(|| raw.as_i64().map(|n| n.to_string()))
        .ok_or_else(|| anyhow::anyhow!("target must be a string or integer"))?;
    let normalized = match typ.as_str() {
        "post" | "forum" => {
            let id: Option<i64> = if typ == "forum" && value.starts_with('/') {
                conn.query_row(
                    "SELECT id FROM objects WHERE kind='forum' AND path=?1",
                    [&value],
                    |r| r.get(0),
                )
                .optional()?
            } else {
                conn.query_row(
                    "SELECT id FROM objects WHERE kind=?1 AND id=?2",
                    params![typ, value.parse::<i64>()?],
                    |r| r.get(0),
                )
                .optional()?
            };
            id.ok_or_else(|| anyhow::anyhow!("{typ} {value} does not exist"))?
                .to_string()
        }
        "tag" | "agent" if !value.trim().is_empty() => value,
        _ => bail!("target_type must be post, forum, tag, or agent with a nonempty target"),
    };
    Ok((typ, normalized))
}

pub fn execute(conn: &mut Connection, actor: &str, request: &Request) -> Result<Output> {
    match request.command.as_str() {
        "subscribe" | "unsubscribe" => {
            let (typ, target) = target(conn, &request.args)?;
            let enabled = request.command == "subscribe";
            let inbox = request.args["inbox"].as_bool().unwrap_or(false);
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute("INSERT INTO subscriptions(agent,target_type,target,automatic,enabled,inbox) VALUES (?1,?2,?3,0,?4,?5) ON CONFLICT(agent,target_type,target) DO UPDATE SET enabled=excluded.enabled,automatic=0,inbox=excluded.inbox",params![actor,typ,target,enabled,inbox])?;
            tx.commit()?;
            Ok(Output::one(
                "subscription",
                json!({"target_type":typ,"target":target,"enabled":enabled,"inbox":inbox}),
            ))
        }
        "subscriptions" => {
            let limit = crate::model::limit(&request.args);
            let offset = request.args["offset"]
                .as_u64()
                .unwrap_or(0)
                .min(i64::MAX as u64) as i64;
            let mut statement = conn.prepare("SELECT target_type,target,automatic,enabled,inbox,created_at FROM subscriptions WHERE agent=?1 AND (?2 OR enabled=1) ORDER BY target_type,target LIMIT ?3 OFFSET ?4")?;
            let mut items = statement.query_map(params![actor,request.args["all"].as_bool().unwrap_or(false),(limit+1) as i64,offset],|r|Ok(json!({"target_type":r.get::<_,String>(0)?,"target":r.get::<_,String>(1)?,"automatic":r.get::<_,bool>(2)?,"enabled":r.get::<_,bool>(3)?,"inbox":r.get::<_,bool>(4)?,"created_at":r.get::<_,String>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let more = items.len() > limit;
            items.truncate(limit);
            Ok(Output {
                kind: "subscriptions".into(),
                items,
                more,
                ..Output::default()
            })
        }
        "feed" | "inbox" | "activity" => retrieve(conn, actor, request),
        "wait" => wait(conn, actor, &request.args),
        other => bail!("unknown attention command {other}"),
    }
}

fn retrieve(conn: &Connection, actor: &str, request: &Request) -> Result<Output> {
    let args = &request.args;
    let global = request.command == "activity";
    let limit = crate::model::limit(args);
    let offset = args["offset"].as_u64().unwrap_or(0).min(i64::MAX as u64) as i64;
    let sql = "SELECT e.id,e.actor,e.kind,e.object_id,e.post_id,e.detail,e.created_at,n.id,n.reasons,n.inbox,n.seen_at,o.title FROM events e LEFT JOIN notifications n ON n.event_id=e.id AND n.agent=?1 LEFT JOIN objects o ON o.id=e.object_id WHERE (?2 OR n.id IS NOT NULL) AND (?2 OR ?3 OR n.seen_at IS NULL) AND (?4=0 OR n.inbox=1) AND (?5 IS NULL OR e.kind=?5) AND (?6 IS NULL OR e.actor=?6) AND e.id>?7 AND (?8 IS NULL OR e.object_id=?8 OR e.post_id=?8) ORDER BY e.id DESC LIMIT ?9 OFFSET ?10";
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map(params![actor,global,args["all"].as_bool().unwrap_or(false),request.command=="inbox",args["kind"].as_str(),args["actor"].as_str(),args["since"].as_i64().unwrap_or(0),args["object"].as_i64().or(args["post"].as_i64()),(limit+1) as i64,offset],|r| {
        let detail: String = r.get(5)?;
        let reasons: Option<String> = r.get(8)?;
        Ok(json!({"id":r.get::<_,i64>(0)?,"actor":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"object_id":r.get::<_,Option<i64>>(3)?,"post_id":r.get::<_,Option<i64>>(4)?,"detail":serde_json::from_str::<Value>(&detail).unwrap_or(Value::Null),"created_at":r.get::<_,String>(6)?,"notification_id":r.get::<_,Option<i64>>(7)?,"reasons":reasons.and_then(|s|serde_json::from_str::<Value>(&s).ok()).unwrap_or(json!([])),"inbox":r.get::<_,Option<bool>>(9)?.unwrap_or(false),"seen_at":r.get::<_,Option<String>>(10)?,"title":r.get::<_,Option<String>>(11)?}))
    })?;
    let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let more = items.len() > limit;
    items.truncate(limit);
    let notification_ids = if global {
        vec![]
    } else {
        items
            .iter()
            .filter_map(|item| item["notification_id"].as_i64())
            .collect()
    };
    Ok(Output {
        kind: request.command.clone(),
        items,
        more,
        notification_ids,
        ..Output::default()
    })
}

fn wait(conn: &Connection, actor: &str, args: &Value) -> Result<Output> {
    let timeout = args["timeout"].as_f64().unwrap_or(60.0);
    if !timeout.is_finite() || !(0.0..=86400.0).contains(&timeout) {
        bail!("timeout must be between 0 and 86400 seconds");
    }
    let duration = Duration::from_secs_f64(timeout);
    let poll = Duration::from_millis(args["poll_ms"].as_u64().unwrap_or(250).clamp(10, 5000));
    let dependencies = args["dependencies"].as_i64();
    if let Some(id) = dependencies
        && !conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE object_id=?1)",
            [id],
            |r| r.get::<_, bool>(0),
        )?
    {
        bail!("task {id} does not exist");
    }
    let post = args["post"].as_i64();
    if let Some(id) = post
        && !conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE id=?1 AND kind='post')",
            [id],
            |r| r.get::<_, bool>(0),
        )?
    {
        bail!("post {id} does not exist");
    }
    let forum = if let Some(value) = args.get("forum").filter(|v| !v.is_null()) {
        Some(
            target(conn, &json!({"target_type":"forum","target":value}))?
                .1
                .parse::<i64>()?,
        )
    } else {
        None
    };
    let task_ready = args["task_ready"].as_bool().unwrap_or(false);
    let explicit = task_ready
        || dependencies.is_some()
        || post.is_some()
        || forum.is_some()
        || args["inbox"].as_bool() == Some(true)
        || args["subscriptions"].as_bool() == Some(true);
    let inbox = args["inbox"].as_bool() == Some(true) || !explicit;
    let subscriptions = args["subscriptions"].as_bool() == Some(true) || !explicit;
    let started = Instant::now();
    loop {
        let mut reasons = Vec::new();
        let mut remaining = Vec::new();
        if inbox || subscriptions {
            let count: i64 = conn.query_row("SELECT count(*) FROM notifications WHERE agent=?1 AND seen_at IS NULL AND (?2 OR inbox=1)",params![actor,subscriptions],|r|r.get(0))?;
            if count > 0 {
                reasons.push(json!({"reason":if subscriptions {"notifications"} else {"inbox"},"count":count}));
            }
        }
        if let Some(id) = dependencies {
            let task_status: String =
                conn.query_row("SELECT status FROM tasks WHERE object_id=?1", [id], |r| {
                    r.get(0)
                })?;
            let mut statement = conn.prepare("SELECT t.object_id,t.status FROM dependencies d JOIN tasks t ON t.object_id=d.prerequisite_id WHERE d.task_id=?1 ORDER BY t.object_id")?;
            let statuses = statement
                .query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let cancelled: Vec<i64> = statuses
                .iter()
                .filter(|s| s.1 == "cancelled")
                .map(|s| s.0)
                .collect();
            remaining = statuses
                .iter()
                .filter(|s| s.1 != "done")
                .map(|s| s.0)
                .collect();
            if task_status == "cancelled" || task_status == "done" {
                reasons.push(json!({"reason":format!("task_{task_status}"),"task_id":id}));
            } else if !cancelled.is_empty() {
                reasons.push(
                    json!({"reason":"dependency_cancelled","task_id":id,"cancelled":cancelled}),
                );
            } else if remaining.is_empty() {
                reasons.push(json!({"reason":"dependencies_ready","task_id":id}));
            }
        }
        if task_ready {
            let visible = format!("NOT ({})", crate::content::archive_predicate("o"));
            let sql = format!(
                "SELECT t.object_id FROM tasks t JOIN objects o ON o.id=t.object_id WHERE t.status='open' AND t.owner IS NULL AND {visible} AND NOT EXISTS(SELECT 1 FROM dependencies d JOIN tasks p ON p.object_id=d.prerequisite_id WHERE d.task_id=t.object_id AND p.status!='done') ORDER BY t.object_id LIMIT 1"
            );
            let id: Option<i64> = conn.query_row(&sql, [], |r| r.get(0)).optional()?;
            if let Some(id) = id {
                reasons.push(json!({"reason":"task_ready","task_id":id}));
            }
        }
        if post.is_some() || forum.is_some() {
            let visible = format!("NOT ({})", crate::content::archive_predicate("o"));
            let sql = format!(
                "WITH RECURSIVE descendants(id) AS (SELECT ?3 UNION ALL SELECT o.id FROM objects o JOIN descendants d ON o.parent_id=d.id WHERE o.kind='forum') SELECT o.id FROM objects o LEFT JOIN view_state v ON v.object_id=o.id AND v.agent=?1 WHERE {visible} AND o.revision>COALESCE(v.seen_revision,0) AND ((?2 IS NOT NULL AND (o.id=?2 OR o.parent_id=?2 AND o.kind='comment')) OR (?3 IS NOT NULL AND o.forum_id IN (SELECT id FROM descendants))) ORDER BY o.id LIMIT 50"
            );
            let mut statement = conn.prepare(&sql)?;
            let ids = statement
                .query_map(params![actor, post, forum], |r| r.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !ids.is_empty() {
                reasons.push(json!({"reason":"updates","object_ids":ids}));
            }
        }
        if !reasons.is_empty() {
            return Ok(Output::one(
                "wait",
                json!({"ready":true,"reasons":reasons,"elapsed_ms":started.elapsed().as_millis()}),
            ));
        }
        let elapsed = started.elapsed();
        if elapsed >= duration {
            return Ok(Output::one(
                "wait",
                json!({"ready":false,"reason":"timeout","remaining":remaining,"elapsed_ms":elapsed.as_millis()}),
            ));
        }
        std::thread::sleep(poll.min(duration - elapsed));
    }
}
