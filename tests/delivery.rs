use agentboard::{app, db, model::Request, render};
use serde_json::{Value, json};
use std::process::{Command, Stdio};

fn run(
    conn: &mut rusqlite::Connection,
    actor: &str,
    command: &str,
    args: Value,
) -> agentboard::model::Output {
    app::execute(
        conn,
        actor,
        &Request {
            command: command.into(),
            args,
        },
    )
    .unwrap()
}

#[test]
fn compact_json_does_not_return_body_or_acknowledge_it() {
    let mut conn = db::open(":memory:").unwrap();
    let output = run(
        &mut conn,
        "author",
        "post.create",
        json!({"title":"Title","body":"private to full reads"}),
    );
    let shown = render::render(&output, "json", true, 65536).unwrap();
    let value: Value = serde_json::from_str(&shown.text).unwrap();
    assert!(value["items"][0].get("body").is_none());
    assert_eq!(value["items"][0]["body_omitted"], true);
    app::acknowledge(&mut conn, "author", &shown).unwrap();
    let revision: i64 = conn
        .query_row(
            "SELECT read_revision FROM view_state WHERE agent='author'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revision, 0);
}

#[test]
fn notification_budget_consumes_only_complete_emitted_items() {
    let mut conn = db::open(":memory:").unwrap();
    for i in 0..5 {
        run(
            &mut conn,
            "author",
            "post.create",
            json!({"title":format!("Note {i} @reader"),"body":"hello"}),
        );
    }
    let output = run(&mut conn, "reader", "inbox", json!({}));
    let shown = render::render(&output, "json", false, 1400).unwrap();
    assert!(!shown.notification_ids.is_empty());
    assert!(shown.notification_ids.len() < 5);
    let count = shown.notification_ids.len();
    app::acknowledge(&mut conn, "reader", &shown).unwrap();
    let remaining = run(&mut conn, "reader", "inbox", json!({}));
    assert_eq!(remaining.items.len(), 5 - count);
    assert!(
        remaining
            .notification_ids
            .iter()
            .all(|id| !shown.notification_ids.contains(id))
    );
}

#[test]
fn closed_stdout_does_not_acknowledge_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let mut conn = db::open(&path).unwrap();
    let output = run(
        &mut conn,
        "author",
        "post.create",
        json!({"title":"Large","body":"x".repeat(500_000)}),
    );
    let id = output.items[0]["id"].as_i64().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agentboard"))
        .arg(&path)
        .args([
            "reader",
            "post",
            "show",
            &id.to_string(),
            "--max-bytes",
            "1000000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert!(!child.wait().unwrap().success());
    let seen: i64 = conn
        .query_row(
            "SELECT count(*) FROM view_state WHERE agent='reader'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seen, 0);
    let failed: i64 = conn
        .query_row(
            "SELECT count(*) FROM command_log WHERE agent='reader' AND success=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(failed, 1);
}

#[test]
fn telemetry_does_not_confuse_event_or_query_ids_with_objects() {
    let mut conn = db::open(":memory:").unwrap();
    let post = run(&mut conn, "a", "post.create", json!({"title":"@reader"}));
    let id = post.items[0]["id"].as_i64().unwrap();
    let req = Request {
        command: "inbox".into(),
        args: json!({}),
    };
    let result = app::execute(&mut conn, "reader", &req);
    app::log_command(&conn, "reader", &req, &result, 1).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT object_ids FROM command_log ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), json!([id]));
    let req = Request {
        command: "query".into(),
        args: json!({"sql":"SELECT 999 AS id"}),
    };
    let result = app::execute(&mut conn, "reader", &req);
    app::log_command(&conn, "reader", &req, &result, 1).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT object_ids FROM command_log ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, "[]");
}
