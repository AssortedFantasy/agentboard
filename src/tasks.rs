//! Transactional task state on ordinary posts. Claims never expire implicitly.
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::{
    db, events,
    model::{self, Output, Receipt, Request},
};

pub fn execute(conn: &mut Connection, actor: &str, request: &Request) -> Result<Output> {
    match request.command.as_str() {
        "task.show" => show(conn, &request.args),
        "task.list" | "task.ready" => list(conn, &request.args, request.command == "task.ready"),
        "task.create" | "task.attach" => create(conn, actor, request),
        "task.depend" | "task.block" => dependencies(conn, actor, request),
        "task.claim" | "task.release" | "task.assign" | "task.takeover" | "task.done"
        | "task.cancel" | "task.reopen" => transition(conn, actor, request),
        _ => bail!("unknown task command: {}", request.command),
    }
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn ids(args: &Value, key: &str) -> Result<Vec<i64>> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .with_context(|| format!("{key} must be an array of task IDs"))?;
    values
        .iter()
        .map(|v| {
            v.as_i64()
                .filter(|id| *id > 0)
                .with_context(|| format!("{key} must contain positive task IDs"))
        })
        .collect::<Result<BTreeSet<_>>>()
        .map(|set| set.into_iter().collect())
}

fn state(conn: &Connection, id: i64) -> Result<(String, Option<String>)> {
    conn.query_row(
        "SELECT status,owner FROM tasks WHERE object_id=?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()?
    .with_context(|| format!("#{id} is not a task; use task attach {id} for an existing post"))
}

fn check_revision(conn: &Connection, id: i64, args: &Value) -> Result<()> {
    if let Some(expected) = args.get("expected_revision") {
        let expected = expected
            .as_i64()
            .context("expected_revision must be an integer")?;
        let current: i64 =
            conn.query_row("SELECT revision FROM objects WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
        ensure!(
            current == expected,
            "revision conflict on #{id}: expected {expected}, current {current}; read the latest object and retry"
        );
    }
    Ok(())
}

fn incomplete(conn: &Connection, id: i64) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare("SELECT d.prerequisite_id,t.status FROM dependencies d JOIN tasks t ON t.object_id=d.prerequisite_id WHERE d.task_id=?1 AND t.status!='done' ORDER BY d.prerequisite_id")?;
    Ok(stmt
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?)
}

fn require_ready(conn: &Connection, id: i64) -> Result<()> {
    let pending = incomplete(conn, id)?;
    ensure!(
        pending.is_empty(),
        "task #{id} has incomplete prerequisites: {}; inspect task show {id}",
        pending
            .iter()
            .map(|(id, status)| format!("#{id} ({status})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

fn save_change(conn: &Connection, actor: &str, id: i64, kind: &str, detail: Value) -> Result<()> {
    conn.execute("UPDATE objects SET revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",[id])?;
    conn.execute(
        "UPDATE tasks SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE object_id=?1",
        [id],
    )?;
    db::save_revision(conn, id, actor)?;
    events::emit(conn, actor, kind, id, &detail)?;
    Ok(())
}

fn add_edge(conn: &Connection, task: i64, prerequisite: i64) -> Result<bool> {
    state(conn, task)?;
    state(conn, prerequisite)?;
    ensure!(task != prerequisite, "task #{task} cannot depend on itself");
    let cycle: bool = conn.query_row("WITH RECURSIVE ancestors(id) AS (SELECT ?1 UNION SELECT d.prerequisite_id FROM dependencies d JOIN ancestors a ON d.task_id=a.id) SELECT EXISTS(SELECT 1 FROM ancestors WHERE id=?2)", params![prerequisite,task],|r|r.get(0))?;
    ensure!(
        !cycle,
        "dependency #{task} -> #{prerequisite} would create a cycle; no changes applied"
    );
    Ok(conn.execute(
        "INSERT OR IGNORE INTO dependencies(task_id,prerequisite_id) VALUES(?1,?2)",
        params![task, prerequisite],
    )? > 0)
}

fn create(conn: &mut Connection, actor: &str, request: &Request) -> Result<Output> {
    let args = &request.args;
    let prerequisites = ids(args, "depends_on")?;
    let blocked = ids(args, "blocks")?;
    let owner = args
        .get("owner")
        .map(|v| v.as_str().context("owner must be an agent name"))
        .transpose()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    db::ensure_agent(&tx, actor)?;
    if let Some(owner) = owner {
        db::register_agent(&tx, owner)?;
    }
    let attaching = request.command == "task.attach";
    let id = if attaching {
        let id = model::id(args, "id")?;
        let kind: Option<String> = tx
            .query_row("SELECT kind FROM objects WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        ensure!(
            kind.as_deref() == Some("post"),
            "task state can only be attached to an existing post"
        );
        check_revision(&tx, id, args)?;
        ensure!(
            !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE object_id=?1)",
                [id],
                |r| r.get::<_, bool>(0)
            )?,
            "#{id} already has task state"
        );
        id
    } else {
        crate::content::create_post(&tx, actor, args)?
    };
    tx.execute("INSERT INTO tasks(object_id,status,owner,updated_at) VALUES(?1,?2,?3,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![id,if owner.is_some(){"claimed"}else{"open"},owner])?;
    for &prerequisite in &prerequisites {
        add_edge(&tx, id, prerequisite)?;
    }
    for &task in &blocked {
        add_edge(&tx, task, id)?;
    }
    let detail = json!({"task_id":id,"owner":owner,"depends_on":prerequisites,"blocks":blocked});
    if attaching {
        save_change(&tx, actor, id, "task.attached", detail)?;
    } else {
        db::save_revision(&tx, id, actor)?;
        events::emit(&tx, actor, "task.created", id, &detail)?;
        let references = tx
            .prepare("SELECT target_id FROM links WHERE source_id=?1 ORDER BY target_id")?
            .query_map([id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        crate::content::emit_references(&tx, actor, id, &references)?;
    }
    for prerequisite in prerequisites {
        save_change(
            &tx,
            actor,
            prerequisite,
            "task.dependencies_changed",
            json!({"task_id":prerequisite,"added_blocks":[id],"removed_blocks":[]}),
        )?;
    }
    for task in blocked {
        save_change(
            &tx,
            actor,
            task,
            "task.dependencies_changed",
            json!({"task_id":task,"added":[id],"removed":[]}),
        )?;
    }
    let output = object_output(&tx, id, true)?;
    tx.commit()?;
    Ok(output)
}

fn transition(conn: &mut Connection, actor: &str, request: &Request) -> Result<Output> {
    let args = &request.args;
    let id = model::id(args, "id")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    db::ensure_agent(&tx, actor)?;
    let (status, owner) = state(&tx, id)?;
    check_revision(&tx, id, args)?;
    let mut next_owner = owner.clone();
    let (next_status, kind) = match request.command.as_str() {
        "task.claim" => {
            ensure!(
                status == "open" && owner.is_none(),
                "task #{id} is {status}{}; use explicit task takeover for an existing claim",
                owner
                    .as_ref()
                    .map(|o| format!(" by {o}"))
                    .unwrap_or_default()
            );
            let archived = crate::content::effectively_archived(&tx, id)?;
            ensure!(
                !archived,
                "task #{id} or its containing forum is archived; restore it before claiming"
            );
            require_ready(&tx, id)?;
            next_owner = Some(actor.into());
            ("claimed", "task.claimed")
        }
        "task.assign" | "task.takeover" => {
            ensure!(
                status == "open" || status == "claimed",
                "task #{id} is {status}; reopen it before assigning"
            );
            let target = if request.command == "task.takeover" {
                args.get("owner")
                    .and_then(Value::as_str)
                    .unwrap_or(actor)
                    .to_owned()
            } else {
                model::string(args, "owner")?
            };
            ensure!(
                owner.as_deref().is_none_or(|o| o == actor || o == target)
                    || request.command == "task.takeover"
                    || flag(args, "takeover"),
                "task #{id} is owned by {}; use task takeover to replace its owner explicitly",
                owner.as_deref().unwrap_or_default()
            );
            db::register_agent(&tx, &target)?;
            next_owner = Some(target);
            (
                "claimed",
                if request.command == "task.takeover" {
                    "task.taken_over"
                } else {
                    "task.assigned"
                },
            )
        }
        "task.release" => {
            ensure!(status == "claimed", "task #{id} is {status}, not claimed");
            require_owner(&owner, actor, id, args)?;
            next_owner = None;
            ("open", "task.released")
        }
        "task.done" => {
            ensure!(
                status == "open" || status == "claimed",
                "task #{id} is already {status}"
            );
            require_owner(&owner, actor, id, args)?;
            require_ready(&tx, id)?;
            ("done", "task.done")
        }
        "task.cancel" => {
            ensure!(
                status == "open" || status == "claimed",
                "task #{id} is already {status}; reopen it before cancelling"
            );
            require_owner(&owner, actor, id, args)?;
            ("cancelled", "task.cancelled")
        }
        "task.reopen" => {
            ensure!(
                status == "done" || status == "cancelled",
                "task #{id} is {status}; only completed or cancelled tasks can be reopened"
            );
            next_owner = None;
            ("open", "task.reopened")
        }
        _ => unreachable!(),
    };
    tx.execute(
        "UPDATE tasks SET status=?2,owner=?3 WHERE object_id=?1",
        params![id, next_status, next_owner],
    )?;
    save_change(
        &tx,
        actor,
        id,
        kind,
        json!({"task_id":id,"previous_status":status,"status":next_status,"previous_owner":owner,"owner":next_owner}),
    )?;
    let output = object_output(&tx, id, true)?;
    tx.commit()?;
    Ok(output)
}

fn require_owner(owner: &Option<String>, actor: &str, id: i64, args: &Value) -> Result<()> {
    ensure!(
        owner.as_deref().is_none_or(|o| o == actor) || flag(args, "takeover"),
        "task #{id} is owned by {}; use task takeover first or pass --takeover for an explicit override",
        owner.as_deref().unwrap_or_default()
    );
    Ok(())
}

fn dependencies(conn: &mut Connection, actor: &str, request: &Request) -> Result<Output> {
    let args = &request.args;
    let id = model::id(args, "id")?;
    let related = ids(args, "ids")?;
    ensure!(!related.is_empty(), "provide at least one related task ID");
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    db::ensure_agent(&tx, actor)?;
    state(&tx, id)?;
    check_revision(&tx, id, args)?;
    let removing = flag(args, "remove");
    let mut changed = std::collections::BTreeMap::<i64, (Vec<i64>, Vec<i64>)>::new();
    for other in related {
        let (task, prerequisite) = if request.command == "task.block" {
            (other, id)
        } else {
            (id, other)
        };
        state(&tx, other)?;
        let did_change = if removing {
            tx.execute(
                "DELETE FROM dependencies WHERE task_id=?1 AND prerequisite_id=?2",
                params![task, prerequisite],
            )? > 0
        } else {
            add_edge(&tx, task, prerequisite)?
        };
        if did_change {
            changed.entry(task).or_default().0.push(prerequisite);
            changed.entry(prerequisite).or_default().1.push(task);
        }
    }
    for (&task, (edges, blocks)) in &changed {
        save_change(
            &tx,
            actor,
            task,
            "task.dependencies_changed",
            json!({"task_id":task,"added":if removing{vec![]}else{edges.clone()},"removed":if removing{edges.clone()}else{vec![]},"added_blocks":if removing{vec![]}else{blocks.clone()},"removed_blocks":if removing{blocks.clone()}else{vec![]}}),
        )?;
    }
    let mut output = object_output(&tx, id, true)?;
    if changed.is_empty() {
        output.notices.push("Dependency graph unchanged.".into());
    } else {
        output.notices.push(format!(
            "Updated dependencies for {} task(s).",
            changed.len()
        ));
    }
    tx.commit()?;
    Ok(output)
}

fn object_output(conn: &Connection, id: i64, full: bool) -> Result<Output> {
    state(conn, id)?;
    let mut value = db::get_object(conn, id)?;
    let pending = incomplete(conn, id)?;
    let (status, _) = state(conn, id)?;
    let archived = crate::content::effectively_archived(conn, id)?;
    value["effectively_archived"] = json!(archived);
    value["ready"] = json!(status == "open" && pending.is_empty() && !archived);
    value["blocked_by"] = json!(pending.iter().map(|(id, _)| id).collect::<Vec<_>>());
    value["cancelled_prerequisites"] = json!(
        pending
            .iter()
            .filter(|(_, s)| s == "cancelled")
            .map(|(id, _)| id)
            .collect::<Vec<_>>()
    );
    let mut statement =
        conn.prepare("SELECT task_id FROM dependencies WHERE prerequisite_id=?1 ORDER BY task_id")?;
    value["blocks"] = json!(
        statement
            .query_map([id], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    );
    let revision = value["revision"]
        .as_i64()
        .context("object revision missing")?;
    if !full {
        value
            .as_object_mut()
            .context("object expected")?
            .remove("body");
        value["body_omitted"] = json!(true);
    }
    let mut output = Output::one("task", value);
    output.receipts.push(Receipt {
        object_id: id,
        revision,
        full,
    });
    Ok(output)
}

fn show(conn: &Connection, args: &Value) -> Result<Output> {
    object_output(conn, model::id(args, "id")?, true)
}

fn list(conn: &Connection, args: &Value, ready: bool) -> Result<Output> {
    let mut filters = vec!["1=1".to_owned()];
    let mut values: Vec<rusqlite::types::Value> = Vec::new();
    if !flag(args, "all") {
        filters.push(format!(
            "{}={}",
            crate::content::archive_predicate("o"),
            i64::from(flag(args, "archived"))
        ));
    }
    if ready {
        filters.push(format!("NOT ({}) AND t.status='open' AND NOT EXISTS(SELECT 1 FROM dependencies d JOIN tasks p ON p.object_id=d.prerequisite_id WHERE d.task_id=o.id AND p.status!='done')",crate::content::archive_predicate("o")));
    }
    for (arg, column) in [
        ("status", "t.status"),
        ("owner", "t.owner"),
        ("author", "o.author"),
    ] {
        if let Some(v) = args.get(arg).and_then(Value::as_str) {
            values.push(v.to_owned().into());
            filters.push(format!("{column}=?{}", values.len()));
        }
    }
    if let Some(tag) = args.get("tag").and_then(Value::as_str) {
        values.push(tag.to_owned().into());
        filters.push(format!(
            "EXISTS(SELECT 1 FROM tags WHERE object_id=o.id AND tag=?{})",
            values.len()
        ));
    }
    if let Some(forum) = args.get("forum").and_then(Value::as_str) {
        let path = if forum == "/" {
            "/"
        } else {
            forum.trim_end_matches('/')
        };
        values.push(path.to_owned().into());
        let parameter = values.len();
        if flag(args, "recursive") {
            filters.push(format!("EXISTS(SELECT 1 FROM objects f WHERE f.id=o.forum_id AND (f.path=?{parameter} OR substr(f.path,1,length(?{parameter})+1)=?{parameter}||'/' OR ?{parameter}='/'))"));
        } else {
            filters.push(format!(
                "EXISTS(SELECT 1 FROM objects f WHERE f.id=o.forum_id AND f.path=?{parameter})"
            ));
        }
    }
    let limit = model::limit(args);
    values.push(((limit + 1) as i64).into());
    let limit_param = values.len();
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(i64::MAX as u64) as i64;
    values.push(offset.into());
    let sql = format!(
        "SELECT o.id FROM tasks t JOIN objects o ON o.id=t.object_id WHERE {} ORDER BY CASE WHEN t.status='open' AND NOT EXISTS(SELECT 1 FROM dependencies d JOIN tasks p ON p.object_id=d.prerequisite_id WHERE d.task_id=o.id AND p.status!='done') THEN 0 WHEN t.status='claimed' THEN 1 WHEN t.status='open' THEN 2 ELSE 3 END,o.id LIMIT ?{limit_param} OFFSET ?{}",
        filters.join(" AND "),
        values.len()
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(values), |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut output = Output {
        kind: "tasks".into(),
        more: rows.len() > limit,
        ..Output::default()
    };
    let full = flag(args, "full");
    for id in rows.into_iter().take(limit) {
        let one = object_output(conn, id, full)?;
        output.items.extend(one.items);
        output.receipts.extend(one.receipts);
    }
    if !full && !output.items.is_empty() {
        output
            .notices
            .push("Bodies omitted. Use --full or task show <id>.".into());
    }
    Ok(output)
}
