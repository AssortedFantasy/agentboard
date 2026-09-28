use crate::{
    content, db, events,
    model::{self, Output, Request},
    query,
    render::Rendered,
    tasks,
};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub fn execute(conn: &mut Connection, actor: &str, request: &Request) -> Result<Output> {
    db::ensure_agent(conn, actor)?;
    let read = matches!(
        request.command.as_str(),
        "query"
            | "search"
            | "history"
            | "diff"
            | "updates"
            | "links"
            | "backlinks"
            | "thread"
            | "feed"
            | "inbox"
            | "subscriptions"
            | "activity"
            | "log"
            | "schema"
    ) || request.command.ends_with(".list")
        || request.command.ends_with(".show")
        || request.command == "task.ready";
    if read {
        conn.execute_batch("BEGIN DEFERRED")?;
    }
    let result = dispatch(conn, actor, request);
    if read {
        match &result {
            Ok(_) => conn.execute_batch("COMMIT")?,
            Err(_) => conn.execute_batch("ROLLBACK")?,
        }
    }
    result
}

fn dispatch(conn: &mut Connection, actor: &str, request: &Request) -> Result<Output> {
    let args = &request.args;
    match request.command.as_str() {
        "init" => Ok(Output::one(
            "board",
            json!({"schema_version":db::SCHEMA_VERSION,"message":"Board ready","root_forum":"/"}),
        )),
        "agent.new" => {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let prefix = args["prefix"].as_str().unwrap_or("agent");
            let name = args["name"].as_str().map(str::to_owned).unwrap_or_else(|| {
                format!(
                    "{prefix}-{}",
                    &uuid::Uuid::new_v4().simple().to_string()[..12]
                )
            });
            ensure!(
                !tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM agents WHERE name=?1)",
                    [&name],
                    |r| r.get::<_, bool>(0)
                )?,
                "agent {name} already exists; choose another name or use it directly"
            );
            db::ensure_agent(&tx, &name)?;
            tx.commit()?;
            Ok(Output::one("agents", json!({"name":name})))
        }
        "agent.list" => {
            let limit = model::limit(args);
            let mut stmt=conn.prepare("SELECT name,profile,created_at,last_active FROM agents ORDER BY name LIMIT ?1 OFFSET ?2")?;
            let mut items = stmt
                .query_map(
                    params![(limit + 1) as i64, args["offset"].as_i64().unwrap_or(0)],
                    agent_row,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let more = items.len() > limit;
            items.truncate(limit);
            Ok(Output {
                more,
                ..Output::list("agents", items)
            })
        }
        "agent.show" => {
            let name = args["name"].as_str().unwrap_or(actor);
            let item = conn
                .query_row(
                    "SELECT name,profile,created_at,last_active FROM agents WHERE name=?1",
                    [name],
                    agent_row,
                )
                .optional()?
                .with_context(|| format!("unknown agent {name}"))?;
            Ok(Output::one("agents", item))
        }
        "agent.profile" => {
            let profile = args
                .get("metadata")
                .or_else(|| args.get("profile"))
                .context("profile requires --metadata JSON")?;
            ensure!(
                profile.is_object(),
                "profile metadata must be a JSON object"
            );
            let tx = conn.transaction()?;
            tx.execute(
                "UPDATE agents SET profile=?1 WHERE name=?2",
                params![profile.to_string(), actor],
            )?;
            tx.execute(
                "INSERT INTO agent_revisions(agent,profile) VALUES(?1,?2)",
                params![actor, profile.to_string()],
            )?;
            tx.commit()?;
            Ok(Output::one(
                "agents",
                json!({"name":actor,"profile":profile}),
            ))
        }
        "config.list" => {
            let mut stmt = conn.prepare("SELECT key,value FROM config ORDER BY key")?;
            let items=stmt.query_map([],|r|Ok(json!({"key":r.get::<_,String>(0)?,"value":serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or(Value::Null)})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Output::list("config", items))
        }
        "config.get" => {
            let key = model::string(args, "key")?;
            Ok(Output::one(
                "config",
                json!({"key":key,"value":db::config(conn,&key)?}),
            ))
        }
        "config.set" => {
            let key = model::string(args, "key")?;
            let value = args.get("value").context("missing value")?;
            validate_config(&key, value)?;
            conn.execute("INSERT INTO config(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value.to_string()])?;
            Ok(Output::one("config", json!({"key":key,"value":value})))
        }
        "log" => {
            let limit = model::limit(args);
            let author = args["author"].as_str();
            let mut stmt=conn.prepare("SELECT id,agent,command,args,success,error,duration_ms,object_ids,created_at FROM command_log WHERE (?1 IS NULL OR agent=?1) AND (?4 IS NULL OR command=?4) AND (?5=0 OR success=0) ORDER BY id DESC LIMIT ?2 OFFSET ?3")?;
            let mut items=stmt.query_map(params![author,(limit+1) as i64,args["offset"].as_i64().unwrap_or(0),args["command"].as_str(),args["failed"].as_bool().unwrap_or(false)],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"agent":r.get::<_,String>(1)?,"command":r.get::<_,String>(2)?,"args":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).unwrap_or(Value::Null),"success":r.get::<_,bool>(4)?,"error":r.get::<_,Option<String>>(5)?,"duration_ms":r.get::<_,i64>(6)?,"object_ids":serde_json::from_str::<Value>(&r.get::<_,String>(7)?).unwrap_or(Value::Null),"created_at":r.get::<_,String>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let more = items.len() > limit;
            items.truncate(limit);
            Ok(Output {
                more,
                ..Output::list("log", items)
            })
        }
        "state.show" => {
            let mut stmt=conn.prepare("SELECT object_id,seen_revision,read_revision,seen_at,read_at FROM view_state WHERE agent=?1 AND (?2 IS NULL OR object_id=?2) ORDER BY object_id LIMIT ?3 OFFSET ?4")?;
            let limit = model::limit(args);
            let mut items=stmt.query_map(params![actor,args["id"].as_i64(),(limit+1) as i64,args["offset"].as_i64().unwrap_or(0)],|r|Ok(json!({"object_id":r.get::<_,i64>(0)?,"seen_revision":r.get::<_,i64>(1)?,"read_revision":r.get::<_,i64>(2)?,"seen_at":r.get::<_,Option<String>>(3)?,"read_at":r.get::<_,Option<String>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let more = items.len() > limit;
            items.truncate(limit);
            Ok(Output {
                more,
                ..Output::list("state", items)
            })
        }
        "state.reset" => {
            let changed = conn.execute(
                "DELETE FROM view_state WHERE agent=?1 AND (?2 IS NULL OR object_id=?2)",
                params![actor, args["id"].as_i64()],
            )?;
            Ok(Output::one(
                "state",
                json!({"agent":actor,"reset_objects":changed,"message":"Observation state reset; the next read can return full content"}),
            ))
        }
        "schema" => {
            let mut stmt=conn.prepare("SELECT name,type,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND name NOT LIKE 'object_search_%' ORDER BY type,name")?;
            let items=stmt.query_map([],|r|Ok(json!({"name":r.get::<_,String>(0)?,"type":r.get::<_,String>(1)?,"sql":r.get::<_,Option<String>>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Output::list("schema", items))
        }
        "query" | "search" => query::execute(conn, actor, request),
        "subscribe" | "unsubscribe" | "subscriptions" | "feed" | "inbox" | "wait" | "activity" => {
            events::execute(conn, actor, request)
        }
        name if name.starts_with("task.") => tasks::execute(conn, actor, request),
        _ => content::execute(conn, actor, request),
    }
}

fn agent_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"name":r.get::<_,String>(0)?,"profile":serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or(Value::Null),"created_at":r.get::<_,String>(2)?,"last_active":r.get::<_,String>(3)?}),
    )
}

fn validate_config(key: &str, value: &Value) -> Result<()> {
    match key {
        "limit" => ensure!(
            value.as_u64().is_some_and(|n| (1..=100_000).contains(&n)),
            "limit must be between 1 and 100000"
        ),
        "max_bytes" => ensure!(
            value
                .as_u64()
                .is_some_and(|n| (1024..=100_000_000).contains(&n)),
            "max_bytes must be between 1024 and 100000000"
        ),
        "format" => ensure!(
            value
                .as_str()
                .is_some_and(|s| ["text", "json", "jsonl"].contains(&s)),
            "format must be text, json or jsonl"
        ),
        "compact" => ensure!(value.is_boolean(), "compact must be true or false"),
        "query_ms" | "poll_ms" => ensure!(
            value.as_u64().is_some_and(|n| (1..=60_000).contains(&n)),
            "{key} must be between 1 and 60000"
        ),
        _ => bail!(
            "unknown setting {key}; settings: limit, max_bytes, format, compact, query_ms, poll_ms"
        ),
    }
    Ok(())
}

pub fn acknowledge(conn: &mut Connection, actor: &str, rendered: &Rendered) -> Result<()> {
    let tx = conn.transaction()?;
    for receipt in &rendered.receipts {
        tx.execute("INSERT INTO view_state(agent,object_id,seen_revision,read_revision,seen_at,read_at) VALUES(?1,?2,?3,?4,strftime('%Y-%m-%dT%H:%M:%fZ','now'),CASE WHEN ?4>0 THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') END) ON CONFLICT(agent,object_id) DO UPDATE SET seen_revision=MAX(seen_revision,excluded.seen_revision),read_revision=MAX(read_revision,excluded.read_revision),seen_at=excluded.seen_at,read_at=CASE WHEN excluded.read_revision>0 THEN excluded.read_at ELSE read_at END",params![actor,receipt.object_id,receipt.revision,if receipt.full {receipt.revision}else{0}])?;
    }
    for id in &rendered.notification_ids {
        tx.execute("UPDATE notifications SET seen_at=COALESCE(seen_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')) WHERE id=?1 AND agent=?2",params![id,actor])?;
    }
    tx.commit()?;
    Ok(())
}

fn log_args(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !k.starts_with('_'))
                .map(|(k, v)| {
                    let value = if matches!(k.as_str(), "body" | "summary" | "metadata" | "profile")
                    {
                        json!({"bytes":v.to_string().len(),"payload_omitted":true})
                    } else {
                        log_args(v)
                    };
                    (k.clone(), value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().take(100).map(log_args).collect()),
        Value::String(s) if s.len() > 4096 => json!({"bytes":s.len(),"payload_omitted":true}),
        _ => value.clone(),
    }
}

pub fn log_command(
    conn: &Connection,
    actor: &str,
    request: &Request,
    result: &Result<Output>,
    duration_ms: i64,
) -> Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    if let Ok(output) = result {
        ids.extend(output.receipts.iter().map(|r| r.object_id));
        if request.command != "query" {
            for item in &output.items {
                if matches!(item["kind"].as_str(), Some("post" | "comment" | "forum")) {
                    ids.extend(item["id"].as_i64());
                }
                ids.extend(item["object_id"].as_i64());
                ids.extend(item["post_id"].as_i64());
            }
        }
        if request.command.starts_with("task.") {
            ids.extend(request.args["id"].as_i64());
            for key in ["ids", "depends_on", "blocks"] {
                if let Some(values) = request.args[key].as_array() {
                    ids.extend(values.iter().filter_map(Value::as_i64));
                }
            }
        }
    }
    let ids: Vec<_> = ids.into_iter().collect();
    conn.execute("INSERT INTO command_log(agent,command,args,success,error,duration_ms,object_ids) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![actor,request.command,log_args(&request.args).to_string(),result.is_ok(),result.as_ref().err().map(|e|format!("{e:#}")),duration_ms,json!(ids).to_string()])?;
    Ok(())
}
