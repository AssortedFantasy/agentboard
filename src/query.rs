//! Read-only SQL selection and full-text discovery. Retrieval produces receipts;
//! the output writer is responsible for applying only successfully emitted ones.
use anyhow::{Context, Result, ensure};
use rusqlite::{
    Batch, Connection,
    fallible_iterator::FallibleIterator,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    params_from_iter,
    types::{Value as SqlValue, ValueRef},
};
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};

use crate::{
    content, db,
    model::{Output, Receipt, Request},
};

const DEFAULT_BYTES: usize = 65_536;
const MAX_BYTES: usize = 100_000_000;
const MAX_VALUE_BYTES: i32 = 16_777_216;

pub fn execute(conn: &mut Connection, actor: &str, request: &Request) -> Result<Output> {
    ensure!(
        matches!(request.command.as_str(), "query" | "search"),
        "unknown query command {}",
        request.command
    );
    // Bind caller identity once outside the restricted query. Names are SQL
    // literals here, escaped rather than interpolated as identifiers.
    conn.execute_batch(&format!(
        "DROP VIEW IF EXISTS temp.me; CREATE TEMP VIEW me AS SELECT '{}' AS name;",
        actor.replace('\'', "''")
    ))?;
    let milliseconds = request
        .args
        .get("query_ms")
        .and_then(Value::as_u64)
        .unwrap_or(1000);
    ensure!(
        (1..=300_000).contains(&milliseconds),
        "query_ms must be between 1 and 300000"
    );
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    let old_length = conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, MAX_VALUE_BYTES)?;
    let old_sql = conn.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 1_048_576)?;
    conn.progress_handler(1000, Some(move || Instant::now() >= deadline))?;
    conn.authorizer(Some(read_authorizer))?;
    let result = if request.command == "search" {
        search(conn, &request.args)
    } else {
        sql_query(conn, &request.args)
    };
    // Always restore hooks, including errors and interrupted queries. The caller
    // still needs to write its command log and eventual delivery receipts.
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?;
    conn.progress_handler(0, None::<fn() -> bool>)?;
    conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, old_length)?;
    conn.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, old_sql)?;
    result.with_context(|| {
        format!(
            "{} failed (read-only SQL, {milliseconds} ms execution budget)",
            request.command
        )
    })
}

fn read_authorizer(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }
        AuthAction::Function { function_name } => {
            match function_name.to_ascii_lowercase().as_str() {
                "load_extension" | "writefile" | "readfile" | "eval" | "fts3_tokenizer" => {
                    Authorization::Deny
                }
                _ => Authorization::Allow,
            }
        }
        // FTS5 reads this harmless pragma internally when opening its index.
        AuthAction::Pragma {
            pragma_name,
            pragma_value: None,
        } if pragma_name.eq_ignore_ascii_case("data_version") => Authorization::Allow,
        _ => Authorization::Deny,
    }
}

fn limits(args: &Value) -> Result<(usize, usize, usize)> {
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50);
    ensure!(
        (1..=100_000).contains(&limit),
        "limit must be between 1 and 100000"
    );
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    ensure!(offset <= i64::MAX as u64, "offset is too large");
    let bytes = args
        .get("max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_BYTES as u64);
    ensure!(
        (1..=MAX_BYTES as u64).contains(&bytes),
        "max_bytes must be between 1 and {MAX_BYTES}"
    );
    Ok((limit as usize, offset as usize, bytes as usize))
}

fn sql_query(conn: &Connection, args: &Value) -> Result<Output> {
    let sql = crate::model::string(args, "sql")?;
    let render = args
        .get("render")
        .and_then(Value::as_str)
        .unwrap_or("table");
    ensure!(
        matches!(render, "table" | "post" | "comment" | "task" | "forum"),
        "render must be table, post, comment, task, or forum"
    );
    let (limit, offset, bytes) = limits(args)?;
    // Let SQLite parse statement boundaries, including comments and quoted
    // semicolons. Validate the entire batch before executing its only SELECT.
    let mut batch = Batch::new(conn, &sql);
    let mut statement = batch.next()?.context("SQL query is empty")?;
    ensure!(batch.next()?.is_none(), "only one SQL statement is allowed");
    ensure!(
        statement.readonly() && statement.column_count() > 0,
        "expected one read-only SELECT or WITH query"
    );
    ensure!(
        statement.parameter_count() == 0,
        "query has unbound parameters; use SQL literals or (SELECT name FROM me) for the calling agent"
    );
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    if render != "table" {
        ensure!(
            columns.len() == 1 && columns[0].eq_ignore_ascii_case("id"),
            "object rendering requires exactly one column named id; use SELECT id FROM ... instead of SELECT *"
        );
    }
    let mut unique = std::collections::HashSet::new();
    ensure!(
        columns.iter().all(|name| unique.insert(name)),
        "duplicate SQL column names; give each expression a unique AS alias"
    );
    let mut rows = statement.query([])?;
    for _ in 0..offset {
        if rows.next()?.is_none() {
            break;
        }
    }
    let mut output = Output {
        kind: if render == "table" {
            "query"
        } else {
            "objects"
        }
        .into(),
        ..Output::default()
    };
    let mut used = 0;
    while let Some(row) = rows.next()? {
        if output.items.len() >= limit {
            output.more = true;
            break;
        }
        let (item, receipt) = if render == "table" {
            let mut item = Map::new();
            for (index, name) in columns.iter().enumerate() {
                item.insert(name.clone(), sql_value(row.get_ref(index)?)?);
            }
            (Value::Object(item), None)
        } else {
            let id = row
                .get::<_, i64>(0)
                .context("object id must be a non-null integer")?;
            let (item, receipt) = object(conn, id, render, args)?;
            (item, Some(receipt))
        };
        if !append(&mut output, item, receipt, &mut used, bytes)? {
            break;
        }
    }
    finish(&mut output, render != "table" && !full(args));
    Ok(output)
}

fn search(conn: &Connection, args: &Value) -> Result<Output> {
    let text = crate::model::string(args, "text")?;
    ensure!(!text.trim().is_empty(), "search text must not be empty");
    let (limit, offset, bytes) = limits(args)?;
    let mut sql = String::from(
        "SELECT o.id, snippet(object_search, -1, '[', ']', ' … ', 24) FROM object_search JOIN objects o ON o.id=object_search.rowid LEFT JOIN objects f ON f.id=o.forum_id WHERE object_search MATCH ?",
    );
    let mut parameters = vec![SqlValue::Text(text)];
    if !args.get("all").and_then(Value::as_bool).unwrap_or(false) {
        if args
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            sql.push_str(&format!(" AND {}", content::archive_predicate("o")));
        } else {
            sql.push_str(&format!(" AND NOT {}", content::archive_predicate("o")));
        }
    }
    if let Some(kind) = args.get("kind").and_then(Value::as_str) {
        ensure!(
            matches!(kind, "post" | "comment" | "forum" | "task"),
            "search kind must be post, comment, forum, or task"
        );
        if kind == "task" {
            sql.push_str(" AND EXISTS (SELECT 1 FROM tasks t WHERE t.object_id=o.id)");
        } else {
            sql.push_str(" AND o.kind=?");
            parameters.push(SqlValue::Text(kind.to_owned()));
        }
    }
    if let Some(forum) = args.get("forum").and_then(Value::as_str) {
        if args
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            // Substring comparison avoids treating '%' and '_' in a path as
            // LIKE wildcards, and respects forum path segment boundaries.
            sql.push_str(
                " AND (coalesce(f.path,o.path)=? OR substr(coalesce(f.path,o.path),1,length(?))=?)",
            );
            let prefix = format!("{}/", forum.trim_end_matches('/'));
            parameters.extend([
                SqlValue::Text(forum.to_owned()),
                SqlValue::Text(prefix.clone()),
                SqlValue::Text(prefix),
            ]);
        } else {
            sql.push_str(" AND coalesce(f.path,o.path)=?");
            parameters.push(SqlValue::Text(forum.to_owned()));
        }
    }
    if let Some(tag) = args.get("tag").and_then(Value::as_str) {
        sql.push_str(" AND EXISTS (SELECT 1 FROM tags WHERE object_id=o.id AND tag=?)");
        parameters.push(SqlValue::Text(tag.to_owned()));
    }
    if let Some(author) = args.get("author").and_then(Value::as_str) {
        sql.push_str(" AND o.author=?");
        parameters.push(SqlValue::Text(author.to_owned()));
    }
    sql.push_str(" ORDER BY bm25(object_search,5.0,1.0,2.0), o.id ASC LIMIT ? OFFSET ?");
    parameters.extend([
        SqlValue::Integer(limit as i64 + 1),
        SqlValue::Integer(offset as i64),
    ]);
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query(params_from_iter(parameters.iter()))?;
    let mut output = Output {
        kind: "objects".into(),
        ..Output::default()
    };
    let mut used = 0;
    while let Some(row) = rows.next()? {
        if output.items.len() >= limit {
            output.more = true;
            break;
        }
        let (mut item, receipt) = object(conn, row.get(0)?, "any", args)?;
        if !full(args) {
            item["snippet"] = json!(row.get::<_, String>(1)?);
        }
        if !append(&mut output, item, Some(receipt), &mut used, bytes)? {
            break;
        }
    }
    finish(&mut output, !full(args));
    Ok(output)
}

fn full(args: &Value) -> bool {
    args.get("full").and_then(Value::as_bool).unwrap_or(false)
}

fn object(conn: &Connection, id: i64, kind: &str, args: &Value) -> Result<(Value, Receipt)> {
    let mut item = db::get_object(conn, id)?;
    if kind == "task" {
        ensure!(
            !item.get("task").is_none_or(Value::is_null),
            "#{id} is not a task"
        );
    } else if kind != "any" {
        ensure!(item["kind"] == kind, "#{id} is not a {kind}");
    }
    let revision = item["revision"]
        .as_i64()
        .context("object has no revision")?;
    if !full(args) {
        item.as_object_mut()
            .context("invalid object")?
            .remove("body");
        item["body_omitted"] = json!(true);
    }
    Ok((
        item,
        Receipt {
            object_id: id,
            revision,
            full: full(args),
        },
    ))
}

fn append(
    output: &mut Output,
    item: Value,
    receipt: Option<Receipt>,
    used: &mut usize,
    bytes: usize,
) -> Result<bool> {
    let size = serde_json::to_vec(&item)?.len();
    if size > bytes.saturating_sub(*used) {
        output.more = true;
        output.notices.push(format!("Output byte budget ({bytes}) reached; narrow the projection or increase --max-bytes. {} rows emitted.", output.items.len()));
        return Ok(false);
    }
    *used += size;
    output.items.push(item);
    if let Some(receipt) = receipt {
        output.receipts.push(receipt);
    }
    Ok(true)
}

fn finish(output: &mut Output, omitted: bool) {
    if omitted && !output.items.is_empty() {
        output.notices.push("Bodies omitted; summaries/snippets are discovery only. Use --full for complete content.".into());
    }
    if output.more {
        output.notices.push("Additional results omitted. Increase --limit or advance --offset by the number of emitted rows; use deterministic ORDER BY when paging SQL results.".into());
    }
}

fn sql_value(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => json!(value),
        ValueRef::Real(value) => json!(value),
        ValueRef::Text(value) => {
            json!(std::str::from_utf8(value).context("SQL text is not UTF-8")?)
        }
        ValueRef::Blob(value) => {
            use std::fmt::Write;
            let mut hex = String::with_capacity(value.len() * 2);
            for byte in value {
                write!(&mut hex, "{byte:02x}")?;
            }
            json!({"blob_hex": hex, "bytes": value.len()})
        }
    })
}
